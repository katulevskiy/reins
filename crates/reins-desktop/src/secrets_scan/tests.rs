//! The scanner against a made-up home folder: every source, the walk rules, the budgets, the refusal for agents, and
//! the report's JSON. No test reads the real home folder.

use std::path::{Path, PathBuf};

use data_encoding::{BASE64, BASE64URL_NOPAD};

use super::*;

/// A token of `len` letters and digits after `prefix`, made when the test runs: no secret-looking literal is in the
/// source (so no scanner of this repository flags it).
pub(crate) fn fake(prefix: &str, len: usize) -> String {
    const ALPHABET: &[u8] = b"aB3dE5fG7hJ9kLmN2pQ4rS6tU8vWxYz0cR1";
    let mut out = prefix.to_owned();
    let mut x: u32 = 0x9E37_79B9
        ^ u32::try_from(len).unwrap_or(0)
        ^ (u32::from(prefix.as_bytes().first().copied().unwrap_or(7)) << 8);
    for _ in 0..len {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        out.push(char::from(ALPHABET[usize::try_from(x).unwrap_or(0) % ALPHABET.len()]));
    }
    out
}

/// An OpenSSH private key file (the container only; its secret part is made up), with or without a passphrase.
pub(crate) fn openssh_key(encrypted: bool) -> (String, Vec<u8>) {
    let mut blob = crate::ssh_agent::wire::Writer::new();
    blob.string(b"ssh-ed25519").string(&[7u8; 32]);
    let mut w = crate::ssh_agent::wire::Writer::new();
    w.string(if encrypted {
        b"aes256-ctr".as_slice()
    } else {
        b"none".as_slice()
    })
    .string(b"none")
    .string(b"")
    .u32(1)
    .string(&blob.buf)
    .string(fake("", 64).as_bytes());
    let mut bytes = b"openssh-key-v1\0".to_vec();
    bytes.extend(&w.buf);
    (
        format!("-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n-----END OPENSSH PRIVATE KEY-----\n", BASE64.encode(&bytes)),
        blob.buf,
    )
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// The made-up home folder, and the secret values put in it (none may appear in a report).
pub(crate) struct Fixture {
    pub dir: tempfile::TempDir,
    pub secrets: Vec<String>,
}

impl Fixture {
    pub fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    pub fn options(&self) -> ScanOptions {
        let mut opts = ScanOptions::new(self.home());
        opts.caller = Caller::Person;
        opts.gh_cli = false;
        opts.git_history = false;
        opts
    }
}

fn jwt(role: &str) -> String {
    let part = |json: &str| BASE64URL_NOPAD.encode(json.as_bytes());
    format!("{}.{}.{}", part(r#"{"alg":"HS256"}"#), part(&format!(r#"{{"role":"{role}"}}"#)), fake("", 43))
}

/// Every source the scanner reads, plus what it must leave alone.
pub(crate) fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let mut secrets = Vec::new();
    let mut s = |v: String| {
        secrets.push(v.clone());
        v
    };
    let openai = s(fake("sk-proj-", 48));
    let stripe = s(fake(&["sk", "_live_"].concat(), 24));
    let db = s(fake("", 18));
    let supabase = s(jwt("service_role"));
    write(
        &home.join("dev/acme-web/.env"),
        &format!(
            "OPENAI_API_KEY={openai}\nSTRIPE_SECRET_KEY={stripe}\nDATABASE_URL=postgres://app:{db}@db.acme.dev/app\n\
             SUPABASE_SERVICE_KEY={supabase}\nNEXT_PUBLIC_SITE_URL=https://acme.dev\nLOG_LEVEL=debug\n"
        ),
    );
    std::fs::create_dir_all(home.join("dev/acme-web/.git")).unwrap();
    write(&home.join("dev/acme-web/.env.example"), &format!("OPENAI_API_KEY={}\n", s(fake("sk-proj-", 49))));
    write(
        &home.join("dev/acme-web/node_modules/pkg/.env"),
        &format!("TOKEN={}\n", s(fake(&["gh", "p_"].concat(), 36))),
    );
    let api_key = s(fake("", 32));
    write(&home.join("dev/api/.env.local"), &format!("API_SECRET={api_key}\nPORT=3000\n"));
    write(&home.join("dev/settings-only/.env"), "PORT=3000\nNODE_ENV=development\n");
    // A repository outside the usual folders, found by its .git.
    std::fs::create_dir_all(home.join("clients/shop/.git")).unwrap();
    write(
        &home.join("clients/shop/.env.production"),
        &format!("STRIPE_KEY={}\n", s(fake(&["sk", "_live_"].concat(), 30))),
    );
    // Not a repository, not a project folder: not looked at.
    write(&home.join("Downloads/old/.env"), &format!("OPENAI_API_KEY={}\n", s(fake("sk-proj-", 50))));
    // Too big, binary.
    write(&home.join("dev/acme-web/.env.big"), &format!("A={}\n{}", s(fake("sk-proj-", 51)), "#".repeat(1_100_000)));
    std::fs::create_dir_all(home.join("dev/acme-web")).unwrap();
    std::fs::write(home.join("dev/acme-web/.env.bin"), b"A=\0\x01binary").unwrap();

    let (key, _) = openssh_key(false);
    write(&home.join(".ssh/id_ed25519"), &key);
    write(&home.join(".ssh/id_ed25519.pub"), "ssh-ed25519 AAAA dana@mac\n");
    let (enc, _) = openssh_key(true);
    write(&home.join(".ssh/work_key"), &enc);
    write(&home.join(".ssh/config"), "Host *\n  IdentityFile ~/.ssh/id_ed25519\n");
    write(&home.join(".ssh/known_hosts"), "github.com ssh-ed25519 AAAA\n");

    let gho = s(fake(&["gh", "o_"].concat(), 36));
    write(
        &home.join(".config/gh/hosts.yml"),
        &format!(
            "github.com:\n    user: dana\n    oauth_token: {gho}\n    git_protocol: https\nghe.acme.dev:\n    user: dana\n"
        ),
    );
    let ghp = s(fake(&["gh", "p_"].concat(), 36));
    let glpat = s(fake("glpat-", 20));
    write(&home.join(".git-credentials"), &format!("https://dana:{ghp}@github.com\nhttps://dana:{glpat}@gitlab.com\n"));
    write(&home.join(".npmrc"), &format!("//registry.npmjs.org/:_authToken={}\n", s(fake("npm_", 36))));
    write(
        &home.join(".pypirc"),
        &format!("[pypi]\nusername = __token__\npassword = {}\n", s(fake("pypi-AgEIcHlwaS5vcmc", 60))),
    );
    write(&home.join(".cargo/credentials.toml"), &format!("[registry]\ntoken = \"{}\"\n", s(fake("cio", 32))));
    write(
        &home.join(".docker/config.json"),
        &format!(
            "{{\"auths\": {{\"ghcr.io\": {{\"auth\": \"{}\"}}}}, \"credsStore\": \"desktop\"}}",
            s(BASE64.encode(fake("dana:", 20).as_bytes()))
        ),
    );
    let akid = s(fake(&["AK", "IA"].concat(), 16).to_ascii_uppercase());
    let aws_secret = s(fake("", 40));
    write(
        &home.join(".aws/credentials"),
        &format!(
            "[default]\naws_access_key_id = {akid}\naws_secret_access_key = {aws_secret}\n[prod]\naws_access_key_id = {akid}\naws_secret_access_key = {aws_secret}\n"
        ),
    );
    write(
        &home.join(".config/gcloud/application_default_credentials.json"),
        &format!(
            "{{\"type\": \"authorized_user\", \"client_id\": \"x.apps\", \"client_secret\": \"{}\", \"refresh_token\": \"{}\"}}",
            s(fake("GOCSPX-", 28)),
            s(fake("1//", 60))
        ),
    );
    write(
        &home.join(".kube/config"),
        &format!(
            "apiVersion: v1\nusers:\n- name: admin\n  user:\n    token: {}\n- name: eks\n  user:\n    exec:\n      command: aws\n",
            s(fake("", 40))
        ),
    );
    let shell_key = s(fake("sk-proj-", 52));
    write(
        &home.join(".zshrc"),
        &format!("export EDITOR=vim\nexport PATH=\"$HOME/bin:$PATH\"\nexport OPENAI_API_KEY={shell_key}\n"),
    );
    write(
        &home.join(".netrc"),
        &format!("machine api.heroku.com\n  login dana@acme.dev\n  password {}\n", s(fake("", 36))),
    );
    Fixture {
        dir,
        secrets,
    }
}

fn run(opts: &ScanOptions) -> ScanReport {
    scan(opts, &mut |_| {}).unwrap()
}

fn by_title<'a>(r: &'a ScanReport, title: &str) -> &'a Finding {
    r.findings
        .iter()
        .find(|f| f.title == title)
        .unwrap_or_else(|| panic!("{title} not found in {:?}", r.findings.iter().map(|f| &f.title).collect::<Vec<_>>()))
}

#[test]
fn every_source_is_found_and_no_value_is_in_the_report() {
    let fx = fixture();
    let report = run(&fx.options());
    let titles: Vec<&str> = report.findings.iter().map(|f| f.title.as_str()).collect();
    for want in [
        "acme-web/.env",
        "api/.env.local",
        "shop/.env.production",
        "id_ed25519",
        "work_key",
        "GitHub CLI token",
        "git credentials",
        "npm token",
        "PyPI token",
        "crates.io token",
        "Docker logins",
        "AWS profiles",
        "Google Cloud credentials",
        "kubeconfig",
        "Shell exports",
        "netrc logins",
    ] {
        assert!(titles.contains(&want), "{want} missing from {titles:?}");
    }
    // Only settings: no finding. Not a project: not looked at. Templates and node_modules: never.
    assert!(!titles.iter().any(|t| t.contains("settings-only") || t.contains("old") || t.contains("example")));
    assert!(!report.findings.iter().any(|f| f.path.contains("node_modules")));
    let json = serde_json::to_string(&report).unwrap();
    let debug = format!("{report:?}");
    for secret in &fx.secrets {
        assert!(!leaks(&json, secret) && !leaks(&debug, secret), "a value leaked into the report");
    }
}

/// Whether `text` holds 6 characters in a row of `secret`.
pub(crate) fn leaks(text: &str, secret: &str) -> bool {
    let chars: Vec<char> = secret.chars().collect();
    chars.windows(6).any(|w| text.contains(&w.iter().collect::<String>()))
}

#[test]
fn nothing_that_holds_a_value_prints_it() {
    let fx = fixture();
    let home = fx.home();
    let files = [
        (Source::Dotenv, home.join("dev/acme-web/.env")),
        (Source::SshKey, home.join(".ssh/id_ed25519")),
        (Source::GhCli, home.join(".config/gh/hosts.yml")),
        (Source::GitCredentials, home.join(".git-credentials")),
        (Source::Npm, home.join(".npmrc")),
        (Source::PyPi, home.join(".pypirc")),
        (Source::Cargo, home.join(".cargo/credentials.toml")),
        (Source::Docker, home.join(".docker/config.json")),
        (Source::Aws, home.join(".aws/credentials")),
        (Source::Gcp, home.join(".config/gcloud/application_default_credentials.json")),
        (Source::Kube, home.join(".kube/config")),
        (Source::Shell, home.join(".zshrc")),
        (Source::Netrc, home.join(".netrc")),
    ];
    let mut read = 0;
    for (source, path) in files {
        let entries = read_entries(source, &path).unwrap();
        read += entries.len();
        let shown = format!("{entries:?} {:?}", entries.first());
        for secret in &fx.secrets {
            assert!(!leaks(&shown, secret), "{source:?} printed a value: {shown}");
        }
    }
    assert!(read > 20);
    let (key, _) = openssh_key(false);
    let shown = format!("{:?}", sources::ssh_key(&key));
    assert!(!leaks(&shown, &fake("", 64)), "{shown}");
    // Errors name the file, never its lines.
    let secret = fake("sk-proj-", 60);
    let bad = fx.dir.path().join("bad.env");
    let mut bytes = format!("OPENAI_API_KEY={secret}\n").into_bytes();
    bytes.push(0xff);
    std::fs::write(&bad, &bytes).unwrap();
    let err = read_entries(Source::Dotenv, &bad).unwrap_err();
    assert!(!leaks(&err, &secret), "{err}");
    let err = read_entries(Source::Dotenv, &fx.dir.path().join("missing")).unwrap_err();
    assert!(err.contains("missing"));
    let err = Caller::Agent("CLAUDECODE".to_owned()).check("A secrets scan").unwrap_err();
    assert!(!err.is_empty());
    // A classification says what, never quotes the value.
    for (name, value) in [("OPENAI_API_KEY", secret.as_str()), ("DB_PASSWORD", "hunter2!xyz")] {
        let v = detect::classify(name, value, false);
        assert!(!leaks(&format!("{v:?} {}", v.label()), value));
    }
}

#[test]
fn a_dotenv_file_lists_each_variable_with_its_kind_and_a_masked_preview() {
    let fx = fixture();
    let report = run(&fx.options());
    let f = by_title(&report, "acme-web/.env");
    assert_eq!(f.source, Source::Dotenv);
    assert_eq!(f.project.as_deref(), Some("acme-web"));
    assert_eq!(f.place, "~/dev/acme-web");
    assert_eq!(f.path, "~/dev/acme-web/.env");
    assert_eq!(f.summary, "6 keys");
    let items: Vec<(&str, &str, bool)> =
        f.items.iter().map(|i| (i.name.as_str(), i.label.as_str(), i.suggested)).collect();
    assert_eq!(
        items,
        [
            ("OPENAI_API_KEY", "OpenAI key", true),
            ("STRIPE_SECRET_KEY", "Stripe live key", true),
            ("DATABASE_URL", "Postgres password", true),
            ("SUPABASE_SERVICE_KEY", "Supabase service key", true),
            ("NEXT_PUBLIC_SITE_URL", "not a secret", false),
            ("LOG_LEVEL", "not a secret", false),
        ]
    );
    assert!(f.items[0].preview.starts_with("sk-p") && f.items[0].preview.contains(detect::MASK));
    assert_eq!(f.secrets(), 4);
    assert!(f.risks.contains(&Risk::PlainText));
    assert!(f.id.starts_with("dotenv-") && f.items[0].id == format!("{}/OPENAI_API_KEY", f.id));
}

#[test]
fn ids_are_stable_from_scan_to_scan() {
    let fx = fixture();
    let a = run(&fx.options());
    let b = run(&fx.options());
    let ids = |r: &ScanReport| r.findings.iter().flat_map(|f| f.items.iter().map(|i| i.id.clone())).collect::<Vec<_>>();
    assert_eq!(ids(&a), ids(&b));
    let all = ids(&a);
    let unique: BTreeSet<&String> = all.iter().collect();
    assert_eq!(unique.len(), all.len(), "ids are unique");
}

#[test]
fn known_files_by_group() {
    let fx = fixture();
    let report = run(&fx.options());
    let ssh = by_title(&report, "id_ed25519");
    assert_eq!((ssh.place.as_str(), ssh.summary.as_str()), ("~/.ssh", "SSH key"));
    assert!(ssh.risks.contains(&Risk::NoPassphrase) && !ssh.risks.contains(&Risk::PlainText));
    assert!(ssh.items[0].preview.starts_with("ED25519 SHA256:"));
    assert_eq!(ssh.key_type.as_deref(), Some("ssh-ed25519"));
    assert!(!by_title(&report, "work_key").risks.contains(&Risk::NoPassphrase));
    // The token in the file and the host whose token is in the keychain.
    let gh = by_title(&report, "GitHub CLI token");
    let hosts: Vec<Option<&str>> = gh.items.iter().map(|i| i.group.as_deref()).collect();
    assert_eq!(hosts, [Some("github.com"), Some("ghe.acme.dev")]);
    assert_eq!(gh.items[1].preview, "in the keychain");
    assert_eq!(by_title(&report, "git credentials").summary, "2 hosts");
    let aws = by_title(&report, "AWS profiles");
    assert_eq!(aws.summary, "default, prod");
    assert_eq!(aws.items.len(), 4);
    assert!(aws.items.iter().all(|i| i.suggested));
    assert_eq!(aws.group, Group::CloudAndShell);
    let shell = by_title(&report, "Shell exports");
    assert_eq!(shell.summary, "OPENAI_API_KEY");
    assert_eq!(shell.items.len(), 1, "settings in a shell file are not listed");
    assert_eq!(by_title(&report, "kubeconfig").items.len(), 1);
    assert_eq!(by_title(&report, "Docker logins").items[0].group.as_deref(), Some("ghcr.io"));
    let skipped: Vec<(&str, SkipReason)> = report.skipped.iter().map(|s| (s.path.as_str(), s.reason)).collect();
    assert!(skipped.contains(&("~/.kube/config", SkipReason::Helper)), "{skipped:?}");
    assert!(skipped.contains(&("~/.docker/config.json", SkipReason::Helper)), "{skipped:?}");
}

#[test]
fn big_and_binary_files_and_links_out_of_home_are_skipped() {
    let fx = fixture();
    let home = fx.home();
    let outside = fx.dir.path().join("outside");
    write(&outside.join(".env"), &format!("OPENAI_API_KEY={}\n", fake("sk-proj-", 53)));
    write(&home.join("dev/inside-target/.env.shared"), &format!("API_TOKEN={}\n", fake("", 30)));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(outside.join(".env"), home.join("dev/acme-web/.env.linked")).unwrap();
        std::os::unix::fs::symlink(&outside, home.join("dev/acme-web/linked-dir")).unwrap();
        std::fs::create_dir_all(home.join("dev/linker")).unwrap();
        std::os::unix::fs::symlink(home.join("dev/inside-target/.env.shared"), home.join("dev/linker/.env")).unwrap();
    }
    let report = run(&fx.options());
    let reasons: Vec<(String, SkipReason)> = report.skipped.iter().map(|s| (s.path.clone(), s.reason)).collect();
    assert!(reasons.contains(&("~/dev/acme-web/.env.big".to_owned(), SkipReason::TooBig)), "{reasons:?}");
    assert!(reasons.contains(&("~/dev/acme-web/.env.bin".to_owned(), SkipReason::Binary)), "{reasons:?}");
    #[cfg(unix)]
    {
        assert!(
            reasons.contains(&("~/dev/acme-web/.env.linked".to_owned(), SkipReason::SymlinkOutside)),
            "{reasons:?}"
        );
        assert!(reasons.contains(&("~/dev/acme-web/linked-dir".to_owned(), SkipReason::SymlinkOutside)), "{reasons:?}");
        assert!(!report.findings.iter().any(|f| f.abs_path.starts_with(&outside)));
        // A link inside the home folder is followed; the file is read once.
        let shared = report.findings.iter().filter(|f| f.items.iter().any(|i| i.name == "API_TOKEN")).count();
        assert_eq!(shared, 1);
    }
}

#[cfg(unix)]
#[test]
fn other_users_files_are_skipped() {
    // A file of root's: only meaningful when the tests do not run as root.
    if rustix::process::geteuid().is_root() {
        return;
    }
    let fx = fixture();
    let mut opts = fx.options();
    let theirs = Path::new("/etc/hosts");
    if !theirs.exists() {
        return;
    }
    opts.paths = vec![theirs.to_path_buf()];
    let report = run(&opts);
    assert!(report.skipped.iter().any(|s| s.reason == SkipReason::OtherUser), "{:?}", report.skipped);
    assert!(report.findings.is_empty());
}

#[test]
fn the_walk_stops_at_its_budgets_and_says_so() {
    let fx = fixture();
    let mut opts = fx.options();
    opts.budget.entries = 3;
    let report = run(&opts);
    assert_eq!(report.budget.as_ref().map(|b| b.limit.as_str()), Some("entries"));
    assert!(report.findings.iter().filter(|f| f.source == Source::Dotenv).count() < 3, "the walk stopped early");
    // The known files are still read.
    assert!(report.findings.iter().any(|f| f.source == Source::Aws));
    let mut opts = fx.options();
    opts.budget.time = Duration::ZERO;
    assert_eq!(run(&opts).budget.map(|b| b.limit), Some("time".to_owned()));
    let mut opts = fx.options();
    opts.budget.findings = 1;
    assert_eq!(run(&opts).budget.map(|b| b.limit), Some("findings".to_owned()));
    assert!(run(&fx.options()).budget.is_none());
}

#[test]
fn a_stop_ends_the_scan() {
    let fx = fixture();
    let mut opts = fx.options();
    let stop = Arc::new(AtomicBool::new(true));
    opts.stop = Some(Arc::clone(&stop));
    let report = run(&opts);
    assert!(report.stopped && report.findings.is_empty());
}

#[test]
fn given_paths_are_the_only_ones_scanned() {
    let fx = fixture();
    let mut opts = fx.options();
    opts.paths = vec![fx.home().join("dev/api"), fx.home().join("clients/shop/.env.production")];
    let report = run(&opts);
    let titles: Vec<&str> = report.findings.iter().map(|f| f.title.as_str()).collect();
    assert_eq!(titles, ["api/.env.local", "shop/.env.production"]);
}

#[test]
fn progress_goes_through_every_stage() {
    let fx = fixture();
    let mut seen: Vec<Progress> = Vec::new();
    scan(&fx.options(), &mut |p| seen.push(p.clone())).unwrap();
    let last = seen.last().unwrap();
    assert!(last.stages.values().all(|s| matches!(s, StageState::Done(_))), "{:?}", last.stages);
    assert!((last.fraction - 1.0).abs() < f32::EPSILON);
    assert_eq!(last.stages[&Stage::Ssh], StageState::Done(2));
    assert!(last.found > 10);
    // The first report has everything waiting but the first stage.
    assert_eq!(seen[0].stages[&Stage::Projects], StageState::Running);
    assert_eq!(seen[0].stages[&Stage::Shell], StageState::Waiting);
}

#[test]
fn agents_are_refused() {
    let env = |pairs: &'static [(&'static str, &'static str)]| {
        move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| (*v).to_owned())
    };
    assert_eq!(Caller::from_vars(env(&[])), Caller::Person);
    assert_eq!(
        Caller::from_vars(env(&[("REINS_RUNNER", "agent:claude-code")])),
        Caller::Agent("REINS_RUNNER=agent:claude-code".to_owned())
    );
    assert_eq!(Caller::from_vars(env(&[("CLAUDECODE", "1")])), Caller::Agent("CLAUDECODE".to_owned()));
    assert_eq!(Caller::from_vars(env(&[("GEMINI_CLI", "1")])), Caller::Agent("GEMINI_CLI".to_owned()));
    assert_eq!(Caller::from_vars(env(&[("CODEX_SANDBOX", "seatbelt")])), Caller::Agent("CODEX_SANDBOX".to_owned()));
    assert_eq!(Caller::from_vars(env(&[("CLAUDECODE", "0")])), Caller::Person);
    // A person's runner wins over a marker left in the environment.
    assert_eq!(Caller::from_vars(env(&[("REINS_RUNNER", "person"), ("CLAUDECODE", "1")])), Caller::Person);
    let fx = fixture();
    let mut opts = fx.options();
    opts.caller = Caller::Agent("CLAUDECODE".to_owned());
    let err = scan(&opts, &mut |_| {}).unwrap_err();
    assert!(err.contains("AI agent") && err.contains("CLAUDECODE"), "{err}");
}

#[test]
fn readable_by_agents_follows_the_harnesses_and_the_guard() {
    let fx = fixture();
    let mut opts = fx.options();
    // No AI tool installed: nobody to read it.
    assert!(!run(&opts).findings.iter().any(|f| f.risks.contains(&Risk::ReadableByAgents)));
    // A tool without Reins' hooks reads anything.
    opts.exposure = Exposure {
        agents: vec!["Claude Code".to_owned()],
        hooked: false,
        guard: GuardConfig::default(),
    };
    let report = run(&opts);
    assert!(report.findings.iter().all(|f| f.risks.contains(&Risk::ReadableByAgents)));
    assert_eq!(report.with_risk(Risk::ReadableByAgents), report.findings.len());
    // With the hooks, the guard's file rules protect what they name (`.env`, `.ssh/*`), not the rest.
    opts.exposure.hooked = true;
    let report = run(&opts);
    assert!(!by_title(&report, "acme-web/.env").risks.contains(&Risk::ReadableByAgents));
    assert!(!by_title(&report, "id_ed25519").risks.contains(&Risk::ReadableByAgents));
    assert!(by_title(&report, "Shell exports").risks.contains(&Risk::ReadableByAgents));
    assert!(by_title(&report, "GitHub CLI token").risks.contains(&Risk::ReadableByAgents));
}

#[cfg(unix)]
#[test]
fn world_readable_files_are_flagged() {
    use std::os::unix::fs::PermissionsExt as _;
    let fx = fixture();
    let file = fx.home().join(".aws/credentials");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::set_permissions(fx.home().join(".netrc"), std::fs::Permissions::from_mode(0o600)).unwrap();
    let report = run(&fx.options());
    assert!(by_title(&report, "AWS profiles").risks.contains(&Risk::WorldReadable));
    assert!(!by_title(&report, "netrc logins").risks.contains(&Risk::WorldReadable));
}

#[test]
fn git_history_is_found_when_git_is_there() {
    let fx = fixture();
    let repo = fx.home().join("dev/acme-web");
    std::fs::remove_dir_all(repo.join(".git")).unwrap();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .is_ok_and(|o| o.status.success())
    };
    if !git(&["init", "-q"]) {
        return;
    }
    assert!(git(&["add", "-f", ".env"]) && git(&["commit", "-q", "-m", "oops"]));
    let mut opts = fx.options();
    opts.git_history = true;
    let report = run(&opts);
    let f = by_title(&report, "acme-web/.env");
    assert!(f.risks.contains(&Risk::InGitHistory), "{:?}", f.risks);
    assert_eq!(f.git_commit.as_ref().map(String::len), Some(7));
    assert!(!by_title(&report, "api/.env.local").risks.contains(&Risk::InGitHistory));
}

#[test]
fn the_json_schema_is_stable() {
    let report = ScanReport {
        version: SCHEMA_VERSION,
        started_at: 1_700_000_000,
        took: 1234,
        home: "/home/dana".to_owned(),
        findings: vec![Finding {
            id: "dotenv-0123456789ab".to_owned(),
            source: Source::Dotenv,
            group: Group::Projects,
            title: "acme-web/.env".to_owned(),
            path: "~/dev/acme-web/.env".to_owned(),
            place: "~/dev/acme-web".to_owned(),
            abs_path: PathBuf::from("/home/dana/dev/acme-web/.env"),
            project: Some("acme-web".to_owned()),
            project_root: Some(PathBuf::from("/home/dana/dev/acme-web")),
            git_commit: Some("a1b2c3d".to_owned()),
            key_type: None,
            summary: "2 keys".to_owned(),
            items: vec![
                Item {
                    id: "dotenv-0123456789ab/OPENAI_API_KEY".to_owned(),
                    name: "OPENAI_API_KEY".to_owned(),
                    group: None,
                    line: 1,
                    detector: Some(Detector::OpenAi),
                    label: "OpenAI key".to_owned(),
                    confidence: Confidence::High,
                    preview: "sk-p••••••••a9F2".to_owned(),
                    suggested: true,
                },
                Item {
                    id: "dotenv-0123456789ab/LOG_LEVEL".to_owned(),
                    name: "LOG_LEVEL".to_owned(),
                    group: None,
                    line: 2,
                    detector: None,
                    label: "not a secret".to_owned(),
                    confidence: Confidence::None,
                    preview: "d••••••••".to_owned(),
                    suggested: false,
                },
            ],
            risks: vec![Risk::ReadableByAgents, Risk::InGitHistory, Risk::PlainText],
            suggested: true,
        }],
        skipped: vec![Skipped {
            path: "~/dev/big/.env".to_owned(),
            reason: SkipReason::TooBig,
        }],
        skipped_total: 1,
        budget: Some(BudgetHit {
            limit: "time".to_owned(),
            at: "~/dev/x".to_owned(),
        }),
        stopped: false,
    };
    let expected = serde_json::json!({
        "version": 1,
        "started_at": 1_700_000_000,
        "took": 1234,
        "home": "/home/dana",
        "findings": [{
            "id": "dotenv-0123456789ab",
            "source": "dotenv",
            "group": "projects",
            "title": "acme-web/.env",
            "path": "~/dev/acme-web/.env",
            "place": "~/dev/acme-web",
            "abs_path": "/home/dana/dev/acme-web/.env",
            "project": "acme-web",
            "project_root": "/home/dana/dev/acme-web",
            "git_commit": "a1b2c3d",
            "summary": "2 keys",
            "items": [
                {"id": "dotenv-0123456789ab/OPENAI_API_KEY", "name": "OPENAI_API_KEY", "line": 1, "detector": "openai",
                 "label": "OpenAI key", "confidence": "high", "preview": "sk-p••••••••a9F2", "suggested": true},
                {"id": "dotenv-0123456789ab/LOG_LEVEL", "name": "LOG_LEVEL", "line": 2, "label": "not a secret",
                 "confidence": "none", "preview": "d••••••••", "suggested": false}
            ],
            "risks": ["readable_by_agents", "in_git_history", "plain_text"],
            "suggested": true
        }],
        "skipped": [{"path": "~/dev/big/.env", "reason": "too_big"}],
        "skipped_total": 1,
        "budget": {"limit": "time", "at": "~/dev/x"},
        "stopped": false
    });
    assert_eq!(serde_json::to_value(&report).unwrap(), expected);
    let back: ScanReport = serde_json::from_value(expected).unwrap();
    assert_eq!(back, report);
    // Every kind and risk keeps its name.
    let names: Vec<String> = Source::ALL.iter().map(|s| serde_json::to_string(s).unwrap()).collect();
    assert_eq!(
        names.join(","),
        r#""dotenv","ssh_key","gh_cli","git_credentials","npm","pypi","cargo","docker","aws","gcp","kube","shell","netrc""#
    );
}
