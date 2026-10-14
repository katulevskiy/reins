//! Keys & secrets' scan and move, as the window walks through it: the hero, scanning (what each stage found as it
//! goes), the review (grouped findings, ticks, a file's detail, what happens after), waiting for the phone (the
//! batch's check digits), and what changed. The scan runs in this process (the window is a person's by
//! construction: an agent does not click it), off the UI thread; the move goes through the same import as `reins
//! secrets import`.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use reins_desktop::secrets_import::{self, Receipt};
use reins_desktop::secrets_scan::{Finding, Group, Progress, Risk, ScanReport, Source};

/// Where the flow is.
#[derive(Clone, Debug, Default)]
pub enum Flow {
    /// The Keys & secrets page with its hero.
    #[default]
    Idle,
    Scanning(Scanning),
    Review(Review),
    /// Waiting for the phone to approve the batch.
    Moving(Moving),
    Done(Finished),
}

#[derive(Clone, Debug)]
pub struct Scanning {
    pub progress: Progress,
    pub stop: Arc<AtomicBool>,
}

impl Scanning {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[derive(Clone, Debug)]
pub struct Review {
    pub report: Arc<ScanReport>,
    /// Item ids.
    pub ticked: BTreeSet<String>,
    /// The finding open in its detail.
    pub open: Option<String>,
    pub keep_local: bool,
    /// Why the last move did not start.
    pub error: Option<String>,
}

impl Review {
    #[must_use]
    pub fn new(report: Arc<ScanReport>) -> Self {
        let ticked = secrets_import::select(&report, &[], true).unwrap_or_default();
        Self {
            report,
            ticked,
            open: None,
            keep_local: false,
            error: None,
        }
    }

    /// How many items are ticked.
    #[must_use]
    pub fn count(&self) -> usize {
        self.ticked.len()
    }

    /// How many could be (every item that may be a secret).
    #[must_use]
    pub fn total(&self) -> usize {
        self.report
            .findings
            .iter()
            .flat_map(|f| &f.items)
            .filter(|i| i.confidence > reins_desktop::secrets_scan::Confidence::None)
            .count()
    }

    /// How many of `f`'s items are ticked.
    #[must_use]
    pub fn ticked_in(&self, f: &Finding) -> usize {
        f.items.iter().filter(|i| self.ticked.contains(&i.id)).count()
    }

    /// Ticks or unticks a whole finding (on: its pre-ticked items, or all that may be secrets).
    pub fn toggle_finding(&mut self, id: &str) {
        let Some(f) = self.report.finding(id) else {
            return;
        };
        if self.ticked_in(f) > 0 {
            for i in &f.items {
                self.ticked.remove(&i.id);
            }
        } else if let Ok(ids) = secrets_import::select(&self.report, &[id.to_owned()], false) {
            self.ticked.extend(ids);
        }
    }

    pub fn toggle_item(&mut self, id: &str) {
        if !self.ticked.remove(id) {
            self.ticked.insert(id.to_owned());
        }
    }

    /// The findings of `group`, in the report's order.
    pub fn in_group(&self, group: Group) -> impl Iterator<Item = &Finding> {
        self.report.findings.iter().filter(move |f| f.group == group)
    }
}

/// The batch on its way to the phone.
#[derive(Clone, Debug, Default)]
pub struct Moving {
    pub check: Option<String>,
    /// What goes where: a title, a line under it, the source, how many secrets.
    pub targets: Vec<(String, String, Source, usize)>,
    pub count: usize,
}

#[derive(Clone, Debug)]
pub struct Finished {
    pub receipt: Receipt,
    pub path: Option<PathBuf>,
    /// The originals were deleted from the trash just now, or undone; what it said.
    pub after: Option<String>,
}

/// The risk chips over the review: how many findings have each.
#[must_use]
pub fn risk_counts(report: &ScanReport) -> Vec<(Risk, usize)> {
    [Risk::ReadableByAgents, Risk::InGitHistory, Risk::NoPassphrase]
        .into_iter()
        .map(|r| (r, report.with_risk(r)))
        .filter(|(_, n)| *n > 0)
        .collect()
}

/// What the move does to a file of `source`, in a sentence (the detail's "After the move").
#[must_use]
pub fn after_move(source: Source) -> &'static str {
    match source {
        Source::Dotenv => ".env becomes .env.reins with vault: references. Run the app with reins run -- <command>.",
        Source::SshKey => "The Reins SSH agent signs with it on your phone; once it has, the file goes to the trash.",
        Source::GhCli | Source::GitCredentials => {
            "git goes through Reins for this host; the token leaves the file, and pushes ask your phone."
        }
        Source::Aws | Source::Npm | Source::PyPi | Source::Cargo | Source::Shell => {
            "A run profile gets it from your vault (reins run --profile …); the file no longer holds it."
        }
        Source::Docker | Source::Gcp | Source::Kube | Source::Netrc => {
            "Its tool reads only this file, so it stays on disk too; your vault keeps a copy."
        }
    }
}

/// The lines `.env.reins` will have for `f` (references for the ticked, the rest as they are): the detail's preview.
#[must_use]
pub fn env_reins_preview(f: &Finding, ticked: &BTreeSet<String>) -> Vec<(String, bool)> {
    let project = f.project.clone().unwrap_or_else(|| "project".to_owned());
    let file = f.abs_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let env = secrets_import::env_for(&file);
    f.items
        .iter()
        .map(|i| {
            if ticked.contains(&i.id) {
                (format!("{}=vault:{project}/{env}/{}", i.name, secrets_import::var_name(&i.name)), true)
            } else {
                (format!("{}=… (stays as it is)", i.name), false)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::Demo;

    #[test]
    fn the_review_starts_with_the_suggested_ticks_and_toggles() {
        let report = Arc::new(Demo::secrets_report(std::path::Path::new("/home/dana")));
        let mut r = Review::new(Arc::clone(&report));
        let acme = report.findings[0].clone();
        assert_eq!(r.ticked_in(&acme), 4);
        r.toggle_finding(&acme.id);
        assert_eq!(r.ticked_in(&acme), 0);
        r.toggle_finding(&acme.id);
        assert_eq!(r.ticked_in(&acme), 4);
        r.toggle_item(&acme.items[0].id);
        assert_eq!(r.ticked_in(&acme), 3);
        assert!(r.count() < r.total());
        assert_eq!(risk_counts(&report)[0].0, Risk::ReadableByAgents);
        let preview = env_reins_preview(&acme, &r.ticked);
        assert_eq!(preview[1].0, "STRIPE_SECRET_KEY=vault:acme-web/development/STRIPE_SECRET_KEY");
        assert!(!preview[0].1 && preview[5].0.starts_with("LOG_LEVEL=…"));
    }
}
