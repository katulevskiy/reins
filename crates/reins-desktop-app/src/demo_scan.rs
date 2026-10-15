//! `--demo` with `REINS_DEMO_SCAN=large`: a big made-up scan (36 project `.env` files in 24 repositories, over 300
//! keys, the same names in several files of one project, and the computer's own keys and tokens), with the vault's
//! projects not checked yet because the phone's key has not been checked on this computer: the state a first real
//! scan of a busy computer opens in. Checking the key "lists" a vault with three projects of the same names from
//! elsewhere, so the review asks about each. Nothing here is real or sent.

use std::path::Path;

use reins_desktop::secrets_import::{Existing, VaultProject};
use reins_desktop::secrets_scan::{Confidence, Detector, Finding, Group, Item, Risk, ScanReport, Source, finding_id};

use crate::demo::Demo;

/// The repositories: folder under the home folder, git origin, and the `.env` files in it (relative to the folder).
const REPOS: &[(&str, Option<&str>, &[&str])] = &[
    ("dev/acme-web", Some("github.com/acme/web"), &[".env", ".env.local", ".env.production"]),
    ("dev/api", None, &[".env", ".env.local"]),
    ("dev/voice", Some("github.com/dana/voice"), &[".env", "serv/.env", "bot/.env"]),
    ("dev/honeypot", Some("github.com/dana/honeypot"), &[".env"]),
    ("dev/billing", Some("github.com/acme/billing"), &[".env", ".env.test"]),
    ("dev/dashboard", Some("github.com/acme/dashboard"), &[".env.local"]),
    ("dev/mobile-api", Some("github.com/acme/mobile-api"), &[".env", ".env.production"]),
    ("dev/scraper", None, &[".env"]),
    ("dev/infra", Some("github.com/acme/infra"), &[".env", "terraform/.env"]),
    ("dev/notify", Some("github.com/acme/notify"), &[".env"]),
    ("dev/search", Some("github.com/acme/search"), &[".env", ".env.local"]),
    ("code/blog", Some("github.com/dana/blog"), &[".env"]),
    ("code/side/invoicer", Some("github.com/dana/invoicer"), &[".env"]),
    ("code/side/habit", None, &[".env.local"]),
    ("code/ml/llm-eval", Some("github.com/dana/llm-eval"), &[".env"]),
    ("code/ml/rag-demo", Some("github.com/dana/rag-demo"), &[".env", ".env.local"]),
    ("code/discord-bots", Some("github.com/dana/discord-bots"), &[".env", "music/.env", "mod/.env"]),
    ("code/home-lab", None, &[".env"]),
    ("src/payments", Some("gitlab.com/acme/payments"), &[".env", ".env.production"]),
    ("src/auth", Some("gitlab.com/acme/auth"), &[".env"]),
    ("src/legacy-crm", None, &[".env"]),
    ("src/analytics", Some("gitlab.com/acme/analytics"), &[".env"]),
    ("work/client-a/api", Some("github.com/client-a/api"), &[".env"]),
    ("work/client-b/site", Some("github.com/client-b/site"), &[".env.local"]),
];

/// Names a project's files hold, and what the scan made of each: detector, label, confidence, a value's first four
/// characters.
const NAMES: &[(&str, Option<Detector>, &str, Confidence, &str)] = &[
    ("OPENAI_API_KEY", Some(Detector::OpenAi), "OpenAI key", Confidence::High, "sk-p"),
    ("ANTHROPIC_API_KEY", Some(Detector::Anthropic), "Anthropic key", Confidence::High, "sk-a"),
    ("STRIPE_SECRET_KEY", Some(Detector::StripeLive), "Stripe live key", Confidence::High, "sk_l"),
    ("STRIPE_WEBHOOK_SECRET", Some(Detector::Generic), "Secret", Confidence::Medium, "whse"),
    ("DATABASE_URL", Some(Detector::Postgres), "Postgres password", Confidence::High, "post"),
    ("REDIS_URL", Some(Detector::Redis), "Redis password", Confidence::High, "redi"),
    ("JWT_SECRET", Some(Detector::Generic), "Secret", Confidence::Medium, "Zq8x"),
    ("SESSION_SECRET", Some(Detector::Generic), "Secret", Confidence::Medium, "p0Lk"),
    ("SENTRY_AUTH_TOKEN", Some(Detector::Generic), "Secret", Confidence::Medium, "sntr"),
    ("GITHUB_TOKEN", Some(Detector::GithubPat), "GitHub token", Confidence::High, "ghp_"),
    ("DISCORD_TOKEN", Some(Detector::Generic), "Secret", Confidence::Medium, "MTA4"),
    ("SLACK_BOT_TOKEN", Some(Detector::Slack), "Slack token", Confidence::High, "xoxb"),
    ("SENDGRID_API_KEY", Some(Detector::SendGrid), "SendGrid key", Confidence::High, "SG.x"),
    ("RESEND_API_KEY", Some(Detector::Generic), "Secret", Confidence::Medium, "re_8"),
    ("SUPABASE_SERVICE_KEY", Some(Detector::SupabaseService), "Supabase service key", Confidence::High, "eyJh"),
    ("AWS_SECRET_ACCESS_KEY", Some(Detector::AwsSecretKey), "AWS secret key", Confidence::High, "wJal"),
    ("GOOGLE_MAPS_KEY", Some(Detector::GoogleApiKey), "Google API key", Confidence::High, "AIza"),
    ("HF_TOKEN", Some(Detector::HuggingFace), "Hugging Face token", Confidence::High, "hf_k"),
    ("TWILIO_AUTH_TOKEN", Some(Detector::Generic), "Secret", Confidence::Medium, "8c1f"),
    ("CLOUDFLARE_API_TOKEN", Some(Detector::Generic), "Secret", Confidence::Medium, "cf_T"),
    ("ADMIN_PASSWORD", Some(Detector::Password), "Password", Confidence::Medium, "hunt"),
    ("POSTHOG_KEY", Some(Detector::Generic), "maybe a secret", Confidence::Low, "phc_"),
    ("LOG_LEVEL", None, "not a secret", Confidence::None, "debu"),
    ("PORT", None, "not a secret", Confidence::None, "3000"),
    ("NODE_ENV", None, "not a secret", Confidence::None, "deve"),
    ("PUBLIC_SITE_URL", None, "not a secret", Confidence::None, "http"),
];

/// A small stable number from `s` (FNV-1a), to pick names and masks from without randomness.
fn mix(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// A masked preview: the value's first four characters, dots, four more.
fn masked(head: &str, seed: &str) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    let mut h = mix(seed);
    let tail: String = (0..4)
        .map(|_| {
            let c = ALPHABET[usize::try_from(h % ALPHABET.len() as u64).unwrap_or(0)];
            h /= ALPHABET.len() as u64;
            char::from(c)
        })
        .collect();
    format!("{head}••••••••{tail}")
}

impl Demo {
    /// `REINS_DEMO_SCAN=large`: the review shows the big scan.
    #[must_use]
    pub fn large_scan() -> bool {
        std::env::var("REINS_DEMO_SCAN").is_ok_and(|v| v == "large")
    }

    /// The scan the demo shows: the big one, or Dana's few files.
    #[must_use]
    pub fn scan(home: &Path) -> ScanReport {
        if Self::large_scan() {
            Self::secrets_report_large(home)
        } else {
            Self::secrets_report(home)
        }
    }

    /// What the demo knows of the vault at the end of the scan: with the big scan, nothing yet (the phone's key is
    /// not checked on this computer).
    #[must_use]
    pub fn scan_vault() -> Existing {
        if Self::large_scan() {
            Existing::Unknown(crate::secrets::KEY_UNCHECKED.to_owned())
        } else {
            Self::secrets_vault()
        }
    }

    /// What the pretend phone lists as the vault's projects.
    #[must_use]
    pub fn listed_vault() -> Existing {
        if Self::large_scan() {
            Self::large_vault()
        } else {
            Self::secrets_vault()
        }
    }

    /// The big scan's vault once the phone's key is checked: three projects of the same names, from elsewhere.
    #[must_use]
    pub fn large_vault() -> Existing {
        let p = |name: &str, origin: Option<&str>| VaultProject {
            name: name.to_owned(),
            origin: origin.map(str::to_owned),
        };
        Existing::Known(vec![
            p("acme-web", Some("github.com/acme/web")),
            p("api", Some("github.com/acme/api")),
            p("dana-voice", Some("github.com/dana/voice-old")),
            p("dana-blog", None),
            p("acme-billing", Some("github.com/acme/billing")),
        ])
    }

    /// The big scan: every repository's `.env` files, then Dana's keys, tokens, cloud and shell (from the small one).
    #[must_use]
    pub fn secrets_report_large(home: &Path) -> ScanReport {
        let mut findings = Vec::new();
        for (r, (folder, origin, files)) in REPOS.iter().enumerate() {
            let root = home.join(folder);
            let project = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            for (f, rel) in files.iter().enumerate() {
                let abs = root.join(rel);
                let id = finding_id(Source::Dotenv, &abs);
                // Each file a run of the names, where it starts and how many from its place; the shared names make
                // the same variable land in two files of one project.
                let seed = mix(&format!("{folder}/{rel}"));
                let start = usize::try_from(seed % 9).unwrap_or(0) + usize::from(f != 0);
                let count = 7 + usize::try_from(seed % 7).unwrap_or(0);
                let mut items: Vec<Item> = Vec::new();
                for k in 0..count {
                    let (name, detector, label, confidence, head) = NAMES[(start + k * 2 + r) % NAMES.len()];
                    if items.iter().any(|i| i.name == name) {
                        continue;
                    }
                    items.push(Item {
                        id: format!("{id}/{name}"),
                        name: name.to_owned(),
                        group: None,
                        line: u32::try_from(items.len() + 1).unwrap_or(1),
                        detector,
                        label: label.to_owned(),
                        confidence,
                        preview: masked(head, &format!("{folder}/{rel}/{name}")),
                        suggested: confidence.suggested(),
                        stays_in_file: false,
                    });
                }
                // The nested files of a project all set the bot's token: one name, several files.
                if rel.contains('/') && !items.iter().any(|i| i.name == "DISCORD_TOKEN") {
                    items.push(Item {
                        id: format!("{id}/DISCORD_TOKEN"),
                        name: "DISCORD_TOKEN".to_owned(),
                        group: None,
                        line: u32::try_from(items.len() + 1).unwrap_or(1),
                        detector: Some(Detector::Generic),
                        label: "Secret".to_owned(),
                        confidence: Confidence::Medium,
                        preview: masked("MTA4", &format!("{folder}/{rel}/DISCORD_TOKEN")),
                        suggested: true,
                        stays_in_file: false,
                    });
                }
                let mut risks = vec![Risk::PlainText];
                // Nearly every file is readable by the AI agents here; a few are in git history too.
                if !(folder.starts_with("work/") && f == 0) {
                    risks.push(Risk::ReadableByAgents);
                }
                if seed.is_multiple_of(6) {
                    risks.push(Risk::InGitHistory);
                }
                risks.sort();
                let shown = format!("~/{folder}/{rel}");
                let place = shown.rsplit_once('/').map_or(shown.clone(), |(d, _)| d.to_owned());
                findings.push(Finding {
                    id,
                    source: Source::Dotenv,
                    group: Group::Projects,
                    title: format!("{project}/{rel}"),
                    path: shown,
                    place,
                    project: Some(project.clone()),
                    project_root: Some(root.clone()),
                    origin: origin.map(str::to_owned),
                    git_commit: risks.contains(&Risk::InGitHistory).then(|| format!("{:07x}", seed & 0x0fff_ffff)),
                    key_type: None,
                    summary: format!("{} keys", items.len()),
                    suggested: items.iter().any(|i| i.suggested),
                    abs_path: abs,
                    items,
                    risks,
                });
            }
        }
        let small = Self::secrets_report(home);
        findings.extend(small.findings.into_iter().filter(|f| f.source != Source::Dotenv));
        ScanReport {
            findings,
            skipped_total: 1_284,
            took: 9_800,
            ..small
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use reins_desktop::secrets_import::{self, Names};

    use super::*;

    #[test]
    fn the_large_scan_is_large_and_has_conflicts() {
        let report = Demo::secrets_report_large(Path::new("/home/dana"));
        let files = report.findings.iter().filter(|f| f.source == Source::Dotenv).count();
        let keys: usize = report.findings.iter().map(|f| f.items.len()).sum();
        assert!(files >= 30, "{files} files");
        assert!(keys >= 300, "{keys} keys");
        // Item ids are unique.
        let ids: BTreeSet<&str> = report.findings.iter().flat_map(|f| f.items.iter().map(|i| i.id.as_str())).collect();
        assert_eq!(ids.len(), keys);
        let selected = secrets_import::select(&report, &[], true).unwrap();
        let merge = BTreeSet::new();
        let names = |existing| Names {
            computer: "Dana's MacBook",
            machine: "machine-danas-macbook-0000",
            existing,
            merge: &merge,
        };
        let unknown = Existing::Unknown("x".to_owned());
        let plan = secrets_import::plan(&report, &selected, &names(&unknown));
        assert!(plan.conflicts.len() >= 10, "{} conflicts", plan.conflicts.len());
        assert!(plan.conflicts.iter().any(|c| c.name == "DISCORD_TOKEN" && c.kept.starts_with("voice/serv")));
        let known = Demo::large_vault();
        let plan = secrets_import::plan(&report, &selected, &names(&known));
        assert_eq!(plan.clashes.iter().filter(|c| c.known).count(), 3, "{:?}", plan.clashes);
    }
}
