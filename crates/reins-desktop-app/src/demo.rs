//! `--demo` only: made-up activity, connections, keys and AI tools, shown where this computer has none yet, so the
//! window can be tried (and photographed) without an account or a running daemon, and a pretend phone that approves
//! work sessions. Nothing here is sent anywhere, and none of it is written to disk. `REINS_DEMO_EMPTY=1` leaves the
//! samples out, to see the empty states; `REINS_DEMO_CALM=1` leaves out the request waiting on the phone;
//! `REINS_DEMO_SESSION=1` starts with a work session running.

use std::collections::BTreeMap;
use std::sync::Arc;

use reins_desktop::api_proxy::ApiConfig;
use reins_desktop::control::{ApiInfo, SshKeyInfo};
use reins_desktop::harness::Harness;
use reins_desktop::journal::{Decider, Entry, Kind, Outcome};
use reins_desktop::run::Profile;
use reins_desktop::secrets_import::{Change, Done, Existing, Moved, Receipt, VaultProject};
use reins_desktop::secrets_scan::{
    Confidence, Detector, Finding, Group, Item, Progress, Risk, ScanReport, Source, Stage, StageState, finding_id,
};
use reins_desktop::stats::Connection;
use reins_desktop::work_session::{Request, Session};

use crate::backend::{DaemonState, Snapshot};

/// The samples, timed from when the demo started.
#[derive(Clone, Copy, Debug)]
pub struct Demo {
    /// Unix seconds the demo started.
    base: i64,
    /// No samples: a computer where nothing happened yet.
    empty: bool,
    /// Nothing waiting on the phone (the calm Home).
    calm: bool,
}

/// One made-up request: seconds before the start, kind, source, service, what, detail, outcome, decider, reason,
/// seconds it took.
type Sample = (
    i64,
    Kind,
    Option<&'static str>,
    Option<&'static str>,
    &'static str,
    Option<&'static str>,
    Outcome,
    Decider,
    Option<&'static str>,
    i64,
);

const SAMPLES: &[Sample] = &[
    (
        40,
        Kind::Command,
        Some("Claude Code"),
        Some("command:git push --force*"),
        "Claude Code wants to run: git push --force origin main",
        Some(
            "Command:\ngit push --force origin main\n\nIn: ~/code/acme-web\nRule: git push --force* (Rewriting git history)",
        ),
        Outcome::Waiting,
        Decider::Phone,
        None,
        0,
    ),
    (
        4 * 60,
        Kind::Mcp,
        Some("Codex"),
        Some("gmail_send"),
        "Codex: gmail_send",
        Some("Arguments: to, subject, body"),
        Outcome::Approved,
        Decider::Phone,
        None,
        9,
    ),
    (
        11 * 60,
        Kind::Command,
        Some("Claude Code"),
        Some("command:rm -r"),
        "Claude Code wants to run: rm -rf /",
        Some("Command:\nrm -rf /\n\nIn: ~/code/acme-web\nRule: rm -r (Recursive deletes)"),
        Outcome::Denied,
        Decider::Phone,
        Some("Denied on your phone."),
        6,
    ),
    (
        23 * 60,
        Kind::Git,
        Some("git"),
        Some("github.com/acme/web"),
        "push to acme/web (main)",
        Some("3 commits to main\nfeat: checkout page\nfix: totals rounding\nchore: bump deps"),
        Outcome::Approved,
        Decider::Phone,
        None,
        14,
    ),
    (
        38 * 60,
        Kind::Api,
        None,
        Some("anthropic"),
        "the key for API anthropic",
        Some("Codex calls https://api.anthropic.com through Reins\nSecret: vault:Anthropic/api key\nLease: 900 s"),
        Outcome::TimedOut,
        Decider::Phone,
        Some("No answer from your phone in time."),
        120,
    ),
    (
        52 * 60,
        Kind::Ssh,
        Some("ssh"),
        Some("github.com"),
        "sign in to github.com as git with MacBook key",
        Some("Key: MacBook key (SHA256:q3Vf0bX8mJx2c1R7yTq9oQe4kZ5aH6uN2pL8sD0wE1g)"),
        Outcome::Approved,
        Decider::Phone,
        None,
        5,
    ),
    (
        75 * 60,
        Kind::File,
        Some("Claude Code"),
        Some("file:.env"),
        "Claude Code wants to read .env",
        Some("File: ~/code/acme-web/.env\nRule: .env files"),
        Outcome::Approved,
        Decider::Phone,
        None,
        7,
    ),
    (
        2 * 3_600,
        Kind::Secrets,
        Some("reins run"),
        None,
        "secrets for `./deploy.sh`",
        Some("Command: ./deploy.sh\nSecrets: vault:AWS deploy/username, vault:AWS deploy/password, vault:Prod DB/url"),
        Outcome::Approved,
        Decider::Phone,
        None,
        11,
    ),
    (
        2 * 3_600 + 20 * 60,
        Kind::Mcp,
        Some("Codex"),
        Some("github_create_issue"),
        "Codex: github_create_issue",
        Some("Arguments: repo, title, body"),
        Outcome::Failed,
        Decider::Phone,
        Some("Your phone could not be reached."),
        2,
    ),
    (
        3 * 3_600,
        Kind::Command,
        Some("Gemini CLI"),
        Some("command:git push origin feature/*"),
        "Gemini CLI wants to run: git push origin feature/login",
        Some("Command:\ngit push origin feature/login\n\nAllowed by your rule: git push origin feature/*"),
        Outcome::Allowed,
        Decider::Settings,
        None,
        0,
    ),
    (
        4 * 3_600,
        Kind::Ask,
        Some("Claude Code"),
        None,
        "Deploy the staging branch now?",
        Some("The tests passed. Deploying takes about 4 minutes."),
        Outcome::Approved,
        Decider::Phone,
        None,
        21,
    ),
    (
        26 * 3_600,
        Kind::Git,
        Some("git"),
        Some("github.com/acme/api"),
        "push to acme/api (release/2.4)",
        Some("1 commit to release/2.4\nfix: rate limit headers"),
        Outcome::Denied,
        Decider::Phone,
        Some("Denied on your phone."),
        30,
    ),
    (
        27 * 3_600,
        Kind::Mcp,
        Some("Claude Code"),
        Some("calendar_create_event"),
        "Claude Code: calendar_create_event",
        Some("Arguments: title, start, end, attendees"),
        Outcome::Approved,
        Decider::Phone,
        None,
        12,
    ),
];

impl Demo {
    #[must_use]
    pub fn new(now: i64) -> Self {
        Self {
            base: now,
            empty: false,
            calm: false,
        }
    }

    /// Without the request that waits on the phone.
    #[must_use]
    pub fn calm(self) -> Self {
        Self {
            calm: true,
            ..self
        }
    }

    /// Without the samples (the empty states).
    #[must_use]
    pub fn empty(self) -> Self {
        Self {
            empty: true,
            ..self
        }
    }

    /// The made-up activity log, newest first.
    #[must_use]
    pub fn activity(self) -> Vec<Entry> {
        SAMPLES
            .iter()
            .enumerate()
            .filter(|(_, sample)| !(self.calm && sample.6 == Outcome::Waiting))
            .map(|(i, &(ago, kind, source, service, what, detail, outcome, decider, reason, took))| {
                let at = self.base - ago;
                Entry {
                    id: format!("demo{i}"),
                    at,
                    kind,
                    source: source.map(str::to_owned),
                    service: service.map(str::to_owned),
                    what: what.to_owned(),
                    detail: detail.map(str::to_owned),
                    decider,
                    outcome,
                    reason: reason.map(str::to_owned),
                    ended_at: (!outcome.open()).then_some(at + took),
                }
            })
            .collect()
    }

    #[must_use]
    pub fn connections(self) -> Vec<Connection> {
        let c = |kind: &str, target: &str, count: u64, ago: i64, last: &str| Connection {
            kind: kind.to_owned(),
            target: target.to_owned(),
            count,
            last_at: self.base - ago,
            last: last.to_owned(),
        };
        vec![
            c("api", "openai", 42, 90, "POST 200"),
            c("git", "github.com/acme/web", 14, 23 * 60, "push approved"),
            c("api", "anthropic", 7, 38 * 60, "denied"),
            c("ssh", "github.com", 3, 52 * 60, "signed"),
            c("git", "github.com/acme/api", 5, 2 * 3_600, "read"),
            c("git", "github.com/acme/design-system", 1, 5 * 3_600, "read"),
        ]
    }

    #[must_use]
    pub fn apis() -> Vec<ApiConfig> {
        vec![
            ApiConfig {
                name: "openai".to_owned(),
                base: "https://api.openai.com/v1".to_owned(),
                header: "Authorization: Bearer {secret}".to_owned(),
                secret: "vault:OpenAI/api key".to_owned(),
                lease_secs: 3_600,
            },
            ApiConfig {
                name: "anthropic".to_owned(),
                base: "https://api.anthropic.com".to_owned(),
                header: "x-api-key: {secret}".to_owned(),
                secret: "vault:Anthropic/api key".to_owned(),
                lease_secs: 900,
            },
        ]
    }

    #[must_use]
    pub fn api_info(self) -> Vec<ApiInfo> {
        vec![
            ApiInfo {
                name: "openai".to_owned(),
                base: "https://api.openai.com/v1".to_owned(),
                leased_until: Some(self.base + 41 * 60),
            },
            ApiInfo {
                name: "anthropic".to_owned(),
                base: "https://api.anthropic.com".to_owned(),
                leased_until: None,
            },
        ]
    }

    #[must_use]
    pub fn profiles() -> BTreeMap<String, Profile> {
        let env = [
            ("AWS_ACCESS_KEY_ID", "vault:AWS deploy/username"),
            ("AWS_SECRET_ACCESS_KEY", "vault:AWS deploy/password"),
            ("DATABASE_URL", "vault:Prod DB/url"),
        ];
        BTreeMap::from([(
            "deploy".to_owned(),
            Profile {
                env: env.into_iter().map(|(k, v)| (k.to_owned(), v.to_owned())).collect(),
                purpose: Some("Deploy the web app to production".to_owned()),
            },
        )])
    }

    #[must_use]
    pub fn ssh_keys() -> Vec<SshKeyInfo> {
        vec![
            SshKeyInfo {
                name: "MacBook key".to_owned(),
                fingerprint: "SHA256:q3Vf0bX8mJx2c1R7yTq9oQe4kZ5aH6uN2pL8sD0wE1g".to_owned(),
            },
            SshKeyInfo {
                name: "Deploy key".to_owned(),
                fingerprint: "SHA256:Zt7nB2cQ9xW4eR1yU6iO3pA8sD5fG0hJ2kL7zX4cV9b".to_owned(),
            },
        ]
    }

    /// `REINS_DEMO_SESSION=1`: a work session that started a while ago, 1 h 12 min left.
    #[must_use]
    pub fn work_session(self) -> Session {
        Session {
            reason: "Fix the login bug".to_owned(),
            started_at: self.base - 48 * 60,
            expires_at: self.base + 72 * 60,
            grants: vec!["demo-g1".to_owned(), "demo-g2".to_owned(), "demo-g3".to_owned()],
            allows: vec![
                "push to github.com acme/web, branch feature/login".to_owned(),
                "read acme/web".to_owned(),
                "read Gmail".to_owned(),
            ],
            skipped: Vec::new(),
        }
    }

    /// What the pretend phone approves for `request` at `now`: a grant for each part, as the phone words them.
    #[must_use]
    pub fn approve(request: &Request, now: i64) -> Session {
        let mut allows = Vec::new();
        for b in &request.push {
            allows.push(format!("push to {} {}, branch {}", b.host, b.repo, b.branch));
            allows.push(format!("read {}", b.repo));
        }
        allows.extend(request.read.iter().map(|id| format!("read {}", crate::work::service_name(id))));
        Session {
            reason: request.reason.clone(),
            started_at: now,
            expires_at: now + i64::try_from(request.secs).unwrap_or(0),
            grants: (1..=allows.len()).map(|i| format!("demo-g{i}")).collect(),
            allows,
            skipped: Vec::new(),
        }
    }

    /// Fills in what this computer does not have yet (the real data wins where there is some).
    pub fn fill(self, s: &mut Snapshot, paused: bool) {
        if !matches!(s.daemon, DaemonState::Running { .. }) {
            s.daemon = DaemonState::Running {
                pending: 0,
            };
        }
        if let Ok(config) = s.config.as_ref()
            && !paused
            && s.git_routed.is_empty()
        {
            s.git_routed = config.enabled_hosts().unwrap_or_default().into_iter().map(|h| h.host).collect();
        }
        if self.empty {
            return;
        }
        if s.activity.is_empty() {
            s.activity = Arc::new(self.activity());
        }
        if let Ok(config) = s.config.as_mut() {
            let config = Arc::make_mut(config);
            if config.api.is_empty() {
                config.api = Self::apis();
            }
            if config.run.profiles.is_empty() {
                config.run.profiles = Self::profiles();
            }
        }
        let overview = Arc::make_mut(&mut s.overview);
        if overview.connections.is_empty() {
            overview.connections = self.connections();
        }
        if overview.apis.is_empty() {
            overview.apis = self.api_info();
        }
        if overview.ssh_socket.is_none() {
            // Where the agent's socket is on this kind of computer (macOS has no runtime directory: the state one).
            let socket = if cfg!(target_os = "macos") {
                "/Users/dana/.local/state/reins/ssh-agent.sock"
            } else {
                "/run/user/1000/reins/ssh-agent.sock"
            };
            overview.ssh_socket = Some(socket.to_owned());
        }
        if overview.ssh_keys.is_empty() {
            overview.ssh_keys = Self::ssh_keys();
        }
        if overview.started_at == 0 {
            overview.started_at = self.base - 5 * 3_600;
        }
        // Reins in no AI tool yet: as if it were in two of them, and Cursor installed beside them (the health card's
        // sample "Installed but not connected").
        if !s.harnesses.iter().any(|h| h.added) {
            for row in &mut s.harnesses {
                let sample = matches!(row.harness, Harness::ClaudeCode | Harness::Codex);
                row.found |= sample || row.harness == Harness::Cursor;
                row.added = sample;
                row.complete = sample;
            }
        }
    }

    /// `--demo`: what a scan of Dana's computer found (made up: the demo never scans this computer).
    #[must_use]
    pub fn secrets_report(home: &std::path::Path) -> ScanReport {
        type ItemSpec<'a> = (&'a str, Option<&'a str>, Option<Detector>, &'a str, Confidence, &'a str);
        let finding = |source: Source,
                       rel: &str,
                       title: &str,
                       summary: &str,
                       risks: &[Risk],
                       items: &[ItemSpec<'_>]| {
            let abs = home.join(rel);
            let id = finding_id(source, &abs);
            let items: Vec<Item> = items
                .iter()
                .map(|(name, group, detector, label, confidence, preview)| Item {
                    id: match group {
                        Some(g) => format!("{id}/{g}/{name}"),
                        None => format!("{id}/{name}"),
                    },
                    name: (*name).to_owned(),
                    group: group.map(str::to_owned),
                    line: 1,
                    detector: *detector,
                    label: (*label).to_owned(),
                    confidence: *confidence,
                    preview: (*preview).to_owned(),
                    suggested: confidence.suggested(),
                    // A setting whose value .env.reins cannot hold as it is: it stays in .env.
                    stays_in_file: *name == "WELCOME_BANNER",
                })
                .collect();
            let shown = format!("~/{rel}");
            let place = match source {
                Source::Dotenv | Source::SshKey => shown.rsplit_once('/').map_or(shown.clone(), |(d, _)| d.to_owned()),
                _ => shown.clone(),
            };
            let project_root =
                (source == Source::Dotenv).then(|| abs.parent().map(std::path::Path::to_path_buf)).flatten();
            Finding {
                id,
                source,
                group: Group::of(source),
                title: title.to_owned(),
                path: shown,
                place,
                project: project_root.as_ref().and_then(|r| r.file_name()).map(|n| n.to_string_lossy().into_owned()),
                // acme-web has a GitHub origin; the api folder has none (and the demo vault has another `api`).
                origin: title.starts_with("acme-web/").then(|| "github.com/acme/web".to_owned()),
                project_root,
                abs_path: abs,
                git_commit: risks.contains(&Risk::InGitHistory).then(|| "a1b2c3d".to_owned()),
                key_type: (source == Source::SshKey).then(|| "ssh-ed25519".to_owned()),
                summary: summary.to_owned(),
                suggested: items.iter().any(|i| i.suggested),
                items,
                risks: risks.to_vec(),
            }
        };
        let (h, m, l, n) = (Confidence::High, Confidence::Medium, Confidence::Low, Confidence::None);
        let findings = vec![
            finding(
                Source::Dotenv,
                "dev/acme-web/.env",
                "acme-web/.env",
                "7 keys",
                &[Risk::ReadableByAgents, Risk::InGitHistory, Risk::PlainText],
                &[
                    ("OPENAI_API_KEY", None, Some(Detector::OpenAi), "OpenAI key", h, "sk-p••••••••a9F2"),
                    ("STRIPE_SECRET_KEY", None, Some(Detector::StripeLive), "Stripe live key", h, "sk_l••••••••hQx7"),
                    ("DATABASE_URL", None, Some(Detector::Postgres), "Postgres password", h, "post••••••••/app"),
                    (
                        "SUPABASE_SERVICE_KEY",
                        None,
                        Some(Detector::SupabaseService),
                        "Supabase service key",
                        h,
                        "eyJh••••••••Xk2Q",
                    ),
                    ("NEXT_PUBLIC_SITE_URL", None, None, "not a secret", n, "http••••••••.dev"),
                    ("LOG_LEVEL", None, None, "not a secret", n, "••••••••"),
                    ("WELCOME_BANNER", None, None, "not a secret", n, "Welc••••••••ack!"),
                ],
            ),
            finding(
                Source::Dotenv,
                "dev/api/.env.local",
                "api/.env.local",
                "3 keys",
                &[Risk::ReadableByAgents, Risk::PlainText],
                &[
                    ("API_SECRET", None, Some(Detector::Generic), "Secret", m, "t7Kq••••••••w2Lp"),
                    ("STRIPE_WEBHOOK_SECRET", None, Some(Detector::Generic), "Secret", m, "whse••••••••9dRz"),
                    ("RESEND_API_KEY", None, Some(Detector::Generic), "Secret", m, "re_8••••••••Ub3c"),
                ],
            ),
            finding(
                Source::SshKey,
                ".ssh/id_ed25519",
                "id_ed25519",
                "SSH key",
                &[Risk::NoPassphrase],
                &[(
                    "private key",
                    None,
                    Some(Detector::PrivateKey),
                    "SSH key",
                    h,
                    "ED25519 SHA256:q3Vf0bX8mJx2c1R7yTq9",
                )],
            ),
            finding(
                Source::GhCli,
                ".config/gh/hosts.yml",
                "GitHub CLI token",
                "github.com",
                &[Risk::ReadableByAgents, Risk::PlainText],
                &[(
                    "oauth_token",
                    Some("github.com"),
                    Some(Detector::GithubOauth),
                    "GitHub OAuth token",
                    h,
                    "gho_••••••••4kPz",
                )],
            ),
            finding(
                Source::GitCredentials,
                ".git-credentials",
                "git credentials",
                "2 hosts",
                &[Risk::PlainText],
                &[
                    ("dana", Some("github.com"), Some(Detector::GithubPat), "GitHub token", h, "ghp_••••••••Rk2m"),
                    ("dana", Some("gitlab.com"), Some(Detector::Credential), "Token", h, "glpa••••••••8sJq"),
                ],
            ),
            finding(
                Source::Npm,
                ".npmrc",
                "npm token",
                "registry.npmjs.org",
                &[Risk::PlainText],
                &[(
                    "_authToken",
                    Some("registry.npmjs.org"),
                    Some(Detector::Generic),
                    "maybe a secret",
                    l,
                    "npm_••••••••x8Qe",
                )],
            ),
            finding(
                Source::Aws,
                ".aws/credentials",
                "AWS profiles",
                "default, prod",
                &[Risk::PlainText],
                &[
                    (
                        "aws_access_key_id",
                        Some("default"),
                        Some(Detector::AwsAccessKeyId),
                        "AWS access key ID",
                        h,
                        "AKIA••••••••Q7XM",
                    ),
                    (
                        "aws_secret_access_key",
                        Some("default"),
                        Some(Detector::AwsSecretKey),
                        "AWS secret key",
                        h,
                        "wJal••••••••EKEY",
                    ),
                    (
                        "aws_access_key_id",
                        Some("prod"),
                        Some(Detector::AwsAccessKeyId),
                        "AWS access key ID",
                        h,
                        "AKIA••••••••3NPD",
                    ),
                    (
                        "aws_secret_access_key",
                        Some("prod"),
                        Some(Detector::AwsSecretKey),
                        "AWS secret key",
                        h,
                        "Zp9r••••••••u4Tb",
                    ),
                ],
            ),
            finding(
                Source::Kube,
                ".kube/config",
                "kubeconfig",
                "1 user",
                &[Risk::InKeychain],
                &[("token", Some("admin"), Some(Detector::Generic), "maybe a secret", l, "eyJh••••••••7wQa")],
            ),
            finding(
                Source::Shell,
                ".zshrc",
                "Shell exports",
                "OPENAI_API_KEY, +1",
                &[Risk::ReadableByAgents, Risk::PlainText],
                &[
                    ("OPENAI_API_KEY", None, Some(Detector::OpenAi), "OpenAI key", h, "sk-p••••••••Lm4d"),
                    ("HF_TOKEN", None, Some(Detector::HuggingFace), "Hugging Face token", h, "hf_k••••••••v9Te"),
                ],
            ),
        ];
        ScanReport {
            version: reins_desktop::secrets_scan::SCHEMA_VERSION,
            started_at: reins_desktop::now_unix(),
            took: 3_400,
            home: home.to_string_lossy().into_owned(),
            findings,
            skipped: Vec::new(),
            skipped_total: 41,
            budget: None,
            stopped: false,
        }
    }

    /// `--demo`: Dana's computer's machine project.
    #[must_use]
    pub fn machine() -> String {
        reins_desktop::secrets_import::machine_project("Dana's MacBook", "demo")
    }

    /// `--demo`: the projects the pretend vault has: an `api` from another repository, so the review shows how a
    /// clash is settled.
    #[must_use]
    pub fn secrets_vault() -> Existing {
        Existing::Known(vec![
            VaultProject {
                name: "api".to_owned(),
                origin: Some("github.com/acme/api".to_owned()),
            },
            VaultProject {
                name: "acme-web".to_owned(),
                origin: Some("github.com/acme/web".to_owned()),
            },
        ])
    }

    /// `--demo`: a scan part of the way through.
    #[must_use]
    pub fn secrets_progress(step: usize) -> Progress {
        let stages = Stage::ALL
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let state = match i.cmp(&step) {
                    std::cmp::Ordering::Less => StageState::Done([9, 2, 4, 5, 1, 2][i]),
                    std::cmp::Ordering::Equal => StageState::Running,
                    std::cmp::Ordering::Greater => StageState::Waiting,
                };
                (*s, state)
            })
            .collect();
        let found = [0, 9, 11, 15, 20, 21, 23][step.min(6)];
        #[allow(clippy::cast_precision_loss, reason = "six stages")]
        let fraction = (step as f32 + 0.7) / 6.0;
        Progress {
            stages,
            found,
            at: match step {
                0 => "~/dev/acme-web/node_modules/… (skipped)",
                1 => "~/.ssh/id_ed25519",
                2 => "~/.config/gh/hosts.yml",
                3 => "~/.aws/credentials",
                4 => "~/.npmrc",
                _ => "~/.zshrc",
            }
            .to_owned(),
            fraction: fraction.min(1.0),
        }
    }

    /// `--demo`: what the pretend phone's approval of `report`'s ticked items changed.
    #[must_use]
    pub fn secrets_receipt(
        report: &ScanReport,
        ticked: &std::collections::BTreeSet<String>,
        keep_local: bool,
    ) -> Receipt {
        let moved: Vec<Moved> = report
            .findings
            .iter()
            .flat_map(|f| f.items.iter().filter(|i| ticked.contains(&i.id)).map(move |i| (f, i)))
            .map(|(f, i)| Moved {
                item: i.id.clone(),
                finding: f.id.clone(),
                to: format!("vault:{}", i.name),
                on_disk: true,
                in_trash: !keep_local
                    && !matches!(f.source, Source::Kube | Source::Docker | Source::Gcp | Source::Netrc),
            })
            .collect();
        let chosen = |source: Source| {
            report
                .findings
                .iter()
                .filter(|f| f.source == source && f.items.iter().any(|i| ticked.contains(&i.id)))
                .count()
        };
        let mut done = Vec::new();
        let mut changes = Vec::new();
        if !keep_local {
            let envs = chosen(Source::Dotenv);
            if envs > 0 {
                done.push(Done {
                    source: Source::Dotenv,
                    what: format!(
                        "{envs} .env file{} → .env.reins",
                        if envs == 1 {
                            ""
                        } else {
                            "s"
                        }
                    ),
                    how: "reins run -- <your command>".to_owned(),
                    ok: true,
                });
            }
            if chosen(Source::SshKey) > 0 {
                done.push(Done {
                    source: Source::SshKey,
                    what: "SSH key → the Reins SSH agent".to_owned(),
                    how: "signs on your phone; the file is in the trash".to_owned(),
                    ok: true,
                });
            }
            if chosen(Source::GhCli) + chosen(Source::GitCredentials) > 0 {
                done.push(Done {
                    source: Source::GhCli,
                    what: "gh and git → git through Reins".to_owned(),
                    how: "pushes ask your phone; no token on disk".to_owned(),
                    ok: true,
                });
            }
            if chosen(Source::Aws) > 0 {
                done.push(Done {
                    source: Source::Aws,
                    what: "AWS → run profiles aws-default, aws-prod".to_owned(),
                    how: "reins run --profile aws-prod -- terraform plan".to_owned(),
                    ok: true,
                });
            }
            if chosen(Source::Shell) > 0 {
                done.push(Done {
                    source: Source::Shell,
                    what: "Shell exports → run profile shell".to_owned(),
                    how: "reins run --profile shell -- <command>".to_owned(),
                    ok: true,
                });
            }
            for f in report.findings.iter().filter(|f| f.source == Source::Dotenv || f.source == Source::SshKey) {
                if f.items.iter().any(|i| ticked.contains(&i.id)) {
                    changes.push(Change::Trashed {
                        path: f.abs_path.clone(),
                        trash_id: None,
                    });
                }
            }
        }
        Receipt {
            version: reins_desktop::secrets_import::RECEIPT_VERSION,
            at: reins_desktop::now_unix(),
            computer: "Dana's MacBook".to_owned(),
            keep_local,
            moved,
            changes,
            done,
            notes: Vec::new(),
            pending_keys: Vec::new(),
            undone_at: None,
            purged_at: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use reins_desktop::control::Overview;

    use super::*;
    use crate::backend::HarnessRow;

    fn snapshot() -> Snapshot {
        Snapshot {
            server: None,
            daemon: DaemonState::Stopped,
            git_routed: Vec::new(),
            harnesses: Harness::ALL
                .into_iter()
                .map(|harness| HarnessRow {
                    harness,
                    found: false,
                    added: false,
                    complete: false,
                })
                .collect(),
            fingerprint: None,
            config: Ok(Arc::new(reins_desktop::config::Config::default())),
            status: None,
            overview: Arc::new(Overview::default()),
            activity: Arc::new(Vec::new()),
            work_session: None,
        }
    }

    #[test]
    fn samples_cover_every_outcome_the_window_shows() {
        let demo = Demo::new(1_800_000_000);
        let entries = demo.activity();
        for outcome in [Outcome::Waiting, Outcome::Approved, Outcome::Denied, Outcome::TimedOut, Outcome::Failed] {
            assert!(entries.iter().any(|e| e.outcome == outcome), "{outcome:?}");
        }
        assert!(entries.windows(2).all(|w| w[0].at >= w[1].at), "newest first");
        let ids: std::collections::HashSet<_> = entries.iter().map(|e| &e.id).collect();
        assert_eq!(ids.len(), entries.len());
        // Open ones have no end; ended ones end after they began.
        assert!(entries.iter().all(|e| e.outcome.open() == e.ended_at.is_none()));
        assert!(entries.iter().filter_map(|e| e.ended_at.map(|end| end >= e.at)).all(|ok| ok));
    }

    #[test]
    fn samples_are_valid_settings() {
        let mut config = reins_desktop::config::Config {
            api: Demo::apis(),
            ..reins_desktop::config::Config::default()
        };
        config.run.profiles = Demo::profiles();
        config.validate().unwrap();
        // Every API the overview lists is configured.
        let demo = Demo::new(1_800_000_000);
        assert!(demo.api_info().iter().all(|i| config.api.iter().any(|a| a.name == i.name)));
    }

    #[test]
    fn fill_only_adds_what_is_missing() {
        let demo = Demo::new(1_800_000_000);
        let mut s = snapshot();
        demo.fill(&mut s, false);
        assert!(!s.activity.is_empty() && !s.overview.connections.is_empty());
        assert!(matches!(s.daemon, DaemonState::Running { .. }));
        assert_eq!(s.git_routed, vec!["github.com"]);
        assert!(s.harnesses.iter().any(|h| h.added));
        let config = s.config.as_ref().unwrap();
        assert_eq!((config.api.len(), config.run.profiles.len()), (2, 1));

        let mut real = snapshot();
        let mine = Entry::new(Kind::Ask, "mine");
        real.activity = Arc::new(vec![mine.clone()]);
        demo.fill(&mut real, true);
        assert_eq!(*real.activity, vec![mine]);
        assert!(real.git_routed.is_empty(), "paused: nothing routed");

        assert!(demo.calm().activity().iter().all(|e| e.outcome != Outcome::Waiting));

        let mut bare = snapshot();
        demo.empty().fill(&mut bare, false);
        assert!(bare.activity.is_empty() && bare.overview.connections.is_empty());
        assert!(matches!(bare.daemon, DaemonState::Running { .. }), "the service still looks on");
        assert_eq!(bare.git_routed, vec!["github.com"]);
    }

    #[test]
    fn the_sample_session_runs_and_the_pretend_phone_approves() {
        let now = 1_800_000_000;
        let s = Demo::new(now).work_session();
        assert_eq!(crate::work::left(&s, now), "1 h 12 min");
        assert_eq!(s.allows.len(), 3);
        assert_eq!(s.grants.len(), s.allows.len());
        assert!(s.allows[0].contains("feature/login") && s.allows[2] == "read Gmail");

        // The repositories the demo's connections list are the form's rows.
        let repos =
            crate::work::recent_repos(&Demo::new(now).connections(), &[], &reins_desktop::config::Config::default());
        assert_eq!(repos.first().map(crate::work::Repo::label).as_deref(), Some("github.com/acme/web"));
        let mut form = crate::work::Form::new(repos);
        form.toggle_read("gmail");
        form.branches[0] = "feature/login".to_owned();
        let request = form.request().unwrap();
        let approved = Demo::approve(&request, now);
        assert_eq!(approved.expires_at, now + 7_200);
        assert_eq!(approved.reason, "Focused work");
        assert_eq!(approved.allows, s.allows, "the same words as the sample");
    }
}
