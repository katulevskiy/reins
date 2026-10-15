//! Keys & secrets' scan and move, as the window walks through it: the hero, scanning (what each stage found as it
//! goes), the review (grouped findings, ticks, a file's detail, what happens after), waiting for the phone (the
//! batch's check digits), and what changed. The scan runs in this process (the window is a person's by
//! construction: an agent does not click it), off the UI thread; the move goes through the same import as `reins
//! secrets import`.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use reins_desktop::secrets_import::{self, Clash, ConflictGroup, Existing, Names, Plan, Receipt, Unchecked};
use reins_desktop::secrets_scan::{Confidence, Finding, Group, Progress, Risk, ScanReport, Source};

/// Why the vault's projects are not known while the phone's key has not been checked on this computer: the phone
/// seals its list to that key, so nothing can be read before.
pub const KEY_UNCHECKED: &str = "your phone's key is not checked yet";

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
#[allow(clippy::struct_excessive_bools, reason = "independent things the review shows")]
pub struct Review {
    pub report: Arc<ScanReport>,
    /// Item ids.
    pub ticked: BTreeSet<String>,
    /// The finding open in its detail.
    pub open: Option<String>,
    pub keep_local: bool,
    /// Why the last move did not start.
    pub error: Option<String>,
    /// What the vault has (its projects and their origins), to never merge into one from elsewhere unasked.
    pub existing: Existing,
    /// The vault's projects are being asked for now (the review shows meanwhile; names may still change).
    pub listing: bool,
    /// Projects the person chose to merge into although the vault's came from elsewhere.
    pub merge: BTreeSet<String>,
    /// The phone's key was never checked on this computer: its twelve digits are typed here first.
    pub key: Option<PhoneKey>,
    /// The left side's rows, made once from the report (they do not change with the ticks).
    pub lines: Arc<Vec<Line>>,
    /// The names step's "More": every project's name, and the variables set in two files.
    pub open_names: bool,
    pub open_dups: bool,
}

/// A tick that stands for several items: all of them ticked, some, or none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tick {
    All,
    Some,
    None,
}

/// One row of the review's left side, in order (the list draws only those in view).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    /// The title, the selection's tick and the risks.
    Head,
    /// A group's heading: its tick, how many, and the risk nearly all of it shares (said once there, not per row).
    Group {
        group: Group,
        findings: Vec<String>,
        shared: Option<Risk>,
    },
    /// A repository with several files: its tick and name; its files follow.
    Repo {
        name: String,
        place: String,
        findings: Vec<String>,
        last: bool,
    },
    /// A file (`nested`: under its repository's row; `last`: the group's last row).
    File {
        id: String,
        nested: bool,
        last: bool,
        shared: Option<Risk>,
    },
    /// What the scan skipped, when it did.
    Foot,
}

/// The risk nearly every finding of `found` has (3 or more, at most one in 6 without it): the group's heading says it
/// once, and the rows leave it out.
#[must_use]
pub fn shared_risk(found: &[&Finding]) -> Option<Risk> {
    if found.len() < 3 {
        return None;
    }
    [Risk::ReadableByAgents, Risk::InGitHistory, Risk::NoPassphrase, Risk::WorldReadable].into_iter().find(|r| {
        let n = found.iter().filter(|f| f.risks.contains(r)).count();
        n * 6 >= found.len() * 5
    })
}

/// The review's rows for `report`: each group's heading, then its files, a project's files under its repository when
/// it has several.
#[must_use]
pub fn lines_of(report: &ScanReport) -> Vec<Line> {
    let mut out = vec![Line::Head];
    for group in Group::ALL {
        let found: Vec<&Finding> = report.findings.iter().filter(|f| f.group == group).collect();
        if found.is_empty() {
            continue;
        }
        let shared = shared_risk(&found);
        out.push(Line::Group {
            group,
            findings: found.iter().map(|f| f.id.clone()).collect(),
            shared,
        });
        // Repositories in the order the scan found them, each with its files.
        let mut units: Vec<(Option<&PathBuf>, Vec<&Finding>)> = Vec::new();
        for f in &found {
            let root = f.project_root.as_ref().filter(|_| f.source == Source::Dotenv);
            match units.iter_mut().find(|(r, _)| root.is_some() && *r == root) {
                Some((_, fs)) => fs.push(f),
                None => units.push((root, vec![f])),
            }
        }
        let n = units.len();
        for (i, (root, files)) in units.into_iter().enumerate() {
            let last_unit = i + 1 == n;
            match root {
                Some(root) if files.len() > 1 => {
                    let name = files[0].project.clone().unwrap_or_else(|| files[0].title.clone());
                    out.push(Line::Repo {
                        name,
                        // The repository's folder, as the scan shows places (`~/dev/voice`).
                        place: reins_desktop::secrets_scan::shown(root, std::path::Path::new(&report.home)),
                        findings: files.iter().map(|f| f.id.clone()).collect(),
                        last: false,
                    });
                    let m = files.len();
                    for (j, f) in files.into_iter().enumerate() {
                        out.push(Line::File {
                            id: f.id.clone(),
                            nested: true,
                            last: last_unit && j + 1 == m,
                            shared,
                        });
                    }
                }
                _ => {
                    for (j, f) in files.iter().enumerate() {
                        out.push(Line::File {
                            id: f.id.clone(),
                            nested: false,
                            last: last_unit && j + 1 == files.len(),
                            shared,
                        });
                    }
                }
            }
        }
    }
    if report.budget.is_some() || report.skipped_total > 0 {
        out.push(Line::Foot);
    }
    out
}

/// A file's name within its repository (`voice/serv/.env` → `serv/.env`), for the rows under the repository's.
#[must_use]
pub fn file_in_repo(f: &Finding) -> String {
    match (&f.project, f.title.split_once('/')) {
        (Some(p), Some((head, rest))) if head == p => rest.to_owned(),
        _ => f.title.clone(),
    }
}

/// What the Move button does now, or why it cannot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MoveState {
    /// Move this many (one approval on the phone).
    Ready(usize),
    /// Nothing is ticked.
    NothingSelected,
    /// The phone's key was never checked here: its digits first.
    EnterKey,
    /// Twelve digits typed: the button checks them (one approval on the phone).
    CheckKey,
    /// The phone is asked for its key.
    CheckingKey,
    /// The vault's projects are being listed: the names may still change.
    CheckingVault,
}

/// How a project of the import is named in the vault.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum How {
    /// Its own name.
    AsIs,
    /// The vault could not be checked: a short suffix keeps it apart until it is.
    Suffixed,
    /// The vault has one of that name from elsewhere, kept apart (the person can merge instead).
    Renamed,
    /// Merged into the vault's project of that name, as chosen.
    Merged,
}

/// One project as the names step lists it: the name it gets, split around its own (`personal-` `api`, `voice`
/// `-c063`), its environments and keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Named {
    pub before: String,
    pub base: String,
    pub after: String,
    pub envs: usize,
    pub keys: usize,
    pub how: How,
}

impl Named {
    #[must_use]
    pub fn name(&self) -> String {
        format!("{}{}{}", self.before, self.base, self.after)
    }
}

/// `chosen` around `base`: what was added before and after it (all of `chosen` when it does not hold `base`).
fn around(chosen: &str, base: &str) -> (String, String, String) {
    if let Some(after) = chosen.strip_prefix(base) {
        (String::new(), base.to_owned(), after.to_owned())
    } else if let Some(before) = chosen.strip_suffix(base) {
        (before.to_owned(), base.to_owned(), String::new())
    } else {
        (String::new(), chosen.to_owned(), String::new())
    }
}

/// The names step: what needs a choice (a project the vault has from elsewhere), the vault not checked (once), every
/// project's name, and the variables two files set (grouped by situation).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Naming {
    pub decisions: Vec<Clash>,
    pub unchecked: Option<Unchecked>,
    pub projects: Vec<Named>,
    /// This computer's own credentials: environments and keys.
    pub machine: (usize, usize),
    pub ssh: usize,
    pub tokens: usize,
    pub duplicates: Vec<ConflictGroup>,
}

impl Naming {
    #[must_use]
    pub fn of(plan: &Plan) -> Self {
        let mut projects: Vec<Named> = Vec::new();
        let how_of = |chosen: &str| -> (String, How) {
            if let Some(c) = plan.clashes.iter().find(|c| c.chosen == chosen) {
                return (
                    c.name.clone(),
                    if c.merged() {
                        How::Merged
                    } else {
                        How::Renamed
                    },
                );
            }
            if let Some(c) = plan.unchecked.iter().flat_map(|u| &u.projects).find(|c| c.chosen == chosen) {
                return (
                    c.name.clone(),
                    if c.merged() {
                        How::Merged
                    } else {
                        How::Suffixed
                    },
                );
            }
            (chosen.to_owned(), How::AsIs)
        };
        let mut machine = (0, 0);
        for e in &plan.envs {
            if e.project_root.is_none() {
                machine.0 += 1;
                machine.1 += e.vars.len();
                continue;
            }
            if let Some(p) = projects.iter_mut().find(|p| p.name() == e.project) {
                p.envs += 1;
                p.keys += e.vars.len();
                continue;
            }
            let (base, how) = how_of(&e.project);
            let (before, base, after) = around(&e.project, &base);
            projects.push(Named {
                before,
                base,
                after,
                envs: 1,
                keys: e.vars.len(),
                how,
            });
        }
        Self {
            decisions: plan.clashes.clone(),
            unchecked: plan.unchecked.clone(),
            projects,
            machine,
            ssh: plan.ssh.len(),
            tokens: plan.tokens.len(),
            duplicates: plan.conflict_groups(),
        }
    }

    /// How many variables two files set.
    #[must_use]
    pub fn duplicate_names(&self) -> usize {
        self.duplicates.iter().map(|g| g.names.len()).sum()
    }
}

/// The phone's key digits, as typed, and whether the phone is being asked for it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PhoneKey {
    pub typed: String,
    pub checking: bool,
}

impl PhoneKey {
    /// The digits typed (`4821-9930-1274`, with spaces or not): twelve make a key to check.
    #[must_use]
    pub fn digits(&self) -> Option<String> {
        let d: String = self.typed.chars().filter(char::is_ascii_digit).collect();
        (d.len() == 12).then_some(d)
    }
}

impl Review {
    #[must_use]
    pub fn new(report: Arc<ScanReport>, existing: Existing) -> Self {
        let ticked = secrets_import::select(&report, &[], true).unwrap_or_default();
        let lines = Arc::new(lines_of(&report));
        Self {
            report,
            ticked,
            open: None,
            keep_local: false,
            error: None,
            existing,
            listing: false,
            merge: BTreeSet::new(),
            key: None,
            lines,
            open_names: false,
            open_dups: false,
        }
    }

    /// What Move does now, or why it cannot (said on the button).
    #[must_use]
    pub fn move_state(&self) -> MoveState {
        match &self.key {
            Some(k) if k.checking => MoveState::CheckingKey,
            Some(k) if k.digits().is_some() => MoveState::CheckKey,
            Some(_) => MoveState::EnterKey,
            None if self.listing => MoveState::CheckingVault,
            None if self.count() == 0 => MoveState::NothingSelected,
            None => MoveState::Ready(self.count()),
        }
    }

    /// The tick of several findings at once.
    #[must_use]
    pub fn tick_of(&self, ids: &[String]) -> Tick {
        let (mut on, mut all) = (0, 0);
        for f in ids.iter().filter_map(|id| self.report.finding(id)) {
            let n = self.ticked_in(f);
            on += usize::from(n > 0);
            all += 1;
        }
        match on {
            0 => Tick::None,
            n if n == all => Tick::All,
            _ => Tick::Some,
        }
    }

    /// Ticks or unticks several findings at once: any ticked → none of them; none → each one's suggested items.
    pub fn toggle_findings(&mut self, ids: &[String]) {
        let any = self.tick_of(ids) != Tick::None;
        for id in ids {
            let Some(f) = self.report.finding(id) else {
                continue;
            };
            let on = self.ticked_in(f) > 0;
            if on == any {
                self.toggle_finding(id);
            }
        }
    }

    /// The whole selection's tick, and toggling it (any ticked → none; none → the suggested ones).
    #[must_use]
    pub fn tick_all(&self) -> Tick {
        let ids: Vec<String> = self.report.findings.iter().map(|f| f.id.clone()).collect();
        self.tick_of(&ids)
    }

    pub fn toggle_all(&mut self) {
        if self.ticked.is_empty() {
            self.ticked = secrets_import::select(&self.report, &[], true).unwrap_or_default();
        } else {
            self.ticked.clear();
        }
    }

    /// Merge into the vault's project `name` (from elsewhere), or keep both apart.
    pub fn set_merge(&mut self, name: &str, merge: bool) {
        if merge {
            self.merge.insert(name.to_owned());
        } else {
            self.merge.remove(name);
        }
    }

    /// Where the ticked secrets would go now (with the clashes and conflicts the review shows).
    #[must_use]
    pub fn plan(&self, computer: &str, machine: &str) -> Plan {
        secrets_import::plan(
            &self.report,
            &self.ticked,
            &Names {
                computer,
                machine,
                existing: &self.existing,
                merge: &self.merge,
            },
        )
    }

    /// How many items are ticked.
    #[must_use]
    pub fn count(&self) -> usize {
        self.ticked.len()
    }

    /// How many could be (every item that may be a secret).
    #[must_use]
    pub fn total(&self) -> usize {
        self.report.findings.iter().flat_map(|f| &f.items).filter(|i| i.confidence > Confidence::None).count()
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

/// Whether two plans put the secrets in the same places (environments with the same variables, keys, hosts): the plan
/// the import makes against the vault as it is when moving must be the one the review showed, else nothing is sent.
#[must_use]
pub fn same_places(a: &Plan, b: &Plan) -> bool {
    let envs = |p: &Plan| -> BTreeSet<(String, String)> {
        p.envs.iter().flat_map(|e| e.vars.iter().map(|v| (e.item(), format!("{}={}", v.name, v.item)))).collect()
    };
    let keys = |p: &Plan| -> BTreeSet<String> { p.ssh.iter().map(|s| s.name.clone()).collect() };
    let hosts = |p: &Plan| -> BTreeSet<String> { p.tokens.iter().map(|t| t.host.clone()).collect() };
    envs(a) == envs(b) && keys(a) == keys(b) && hosts(a) == hosts(b)
}

/// The project `f`'s variables go to in `plan` (as named after any clash), else its own name.
#[must_use]
pub fn project_in(plan: &Plan, f: &Finding) -> String {
    plan.envs
        .iter()
        .find(|e| e.project_root.is_some() && e.project_root == f.project_root)
        .map_or_else(|| secrets_import::base_project(f), |e| e.project.clone())
}

/// The lines `.env.reins` will have for `f` in `project` (references for the ticked, the rest as they are): the
/// detail's preview.
#[must_use]
pub fn env_reins_preview(f: &Finding, ticked: &BTreeSet<String>, project: &str) -> Vec<(String, bool)> {
    let file = f.abs_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let env = secrets_import::env_for(&file);
    f.items
        .iter()
        .map(|i| {
            if ticked.contains(&i.id) {
                (format!("{}=vault:{project}/{env}/{}", i.name, secrets_import::var_name(&i.name)), true)
            } else if i.stays_in_file || i.confidence > Confidence::None {
                // .env.reins cannot hold it as it is, or it may be a secret: it stays in its .env file.
                (format!("{} stays in {file}", i.name), false)
            } else {
                (format!("{}=… (as it is)", i.name), false)
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
        let mut r = Review::new(Arc::clone(&report), Demo::secrets_vault());
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
        let plan = r.plan("Dana's MacBook", "machine-danas-macbook-0000");
        let preview = env_reins_preview(&acme, &r.ticked, &project_in(&plan, &acme));
        assert_eq!(preview[1].0, "STRIPE_SECRET_KEY=vault:acme-web/development/STRIPE_SECRET_KEY");
        assert!(!preview[0].1 && preview[5].0.starts_with("LOG_LEVEL=…"));
        assert_eq!(preview[0].0, "OPENAI_API_KEY stays in .env", "unticked, it may be a secret");
        // The demo vault has an `api` from another folder: this one gets a new name, unless merged on purpose.
        let api = report.findings.iter().find(|f| f.title == "api/.env.local").unwrap();
        assert_eq!(project_in(&plan, api), "dev-api");
        assert_eq!(plan.clashes.len(), 1);
        r.set_merge("api", true);
        assert_eq!(project_in(&r.plan("x", "machine-x-0000"), api), "api");
        r.set_merge("api", false);
        assert_eq!(project_in(&r.plan("x", "machine-x-0000"), api), "dev-api");
    }

    fn large() -> Review {
        let report = Arc::new(Demo::secrets_report_large(std::path::Path::new("/home/dana")));
        Review::new(report, Existing::Unknown(KEY_UNCHECKED.to_owned()))
    }

    #[test]
    fn a_large_scan_groups_files_by_repository_and_says_a_shared_risk_once() {
        let r = large();
        let lines = &r.lines;
        assert_eq!(lines[0], Line::Head);
        // Every finding has exactly one row.
        let files = lines.iter().filter(|l| matches!(l, Line::File { .. })).count();
        assert_eq!(files, r.report.findings.len());
        // A repository with several files gets a row, its files under it.
        let voice = lines
            .iter()
            .position(|l| matches!(l, Line::Repo { name, .. } if name == "voice"))
            .expect("voice has three files");
        let Line::Repo {
            findings,
            place,
            ..
        } = &lines[voice]
        else {
            unreachable!()
        };
        assert_eq!(findings.len(), 3);
        assert_eq!(place, "~/dev/voice");
        assert!(lines[voice + 1..=voice + 3].iter().all(|l| matches!(
            l,
            Line::File {
                nested: true,
                ..
            }
        )));
        // A one-file repository is just its file.
        let honeypot = r.report.findings.iter().find(|f| f.title == "honeypot/.env").unwrap();
        assert!(lines.iter().any(|l| matches!(l, Line::File { id, nested: false, .. } if *id == honeypot.id)));
        // Nearly every project file is readable by AI agents: said once, on the group.
        assert!(lines.iter().any(|l| matches!(
            l,
            Line::Group {
                group: Group::Projects,
                shared: Some(Risk::ReadableByAgents),
                ..
            }
        )));
        let serv = r.report.findings.iter().find(|f| f.title == "voice/serv/.env").unwrap();
        assert_eq!(file_in_repo(serv), "serv/.env");
        // The few files of a small group say their own risks.
        let small = Demo::secrets_report(std::path::Path::new("/home/dana"));
        let found: Vec<&Finding> = small.findings.iter().filter(|f| f.group == Group::Projects).collect();
        assert_eq!(shared_risk(&found), None);
    }

    #[test]
    fn group_ticks_select_all_some_or_none() {
        let mut r = large();
        let Some(Line::Group {
            findings,
            ..
        }) = r.lines.get(1).cloned()
        else {
            panic!("{:?}", r.lines[1])
        };
        assert_eq!(r.tick_of(&findings), Tick::All);
        r.toggle_finding(&findings[0]);
        assert_eq!(r.tick_of(&findings), Tick::Some);
        r.toggle_findings(&findings);
        assert_eq!(r.tick_of(&findings), Tick::None);
        r.toggle_findings(&findings);
        assert_eq!(r.tick_of(&findings), Tick::All);
        // The kubeconfig's token is only maybe a secret: not ticked unless picked, so the whole is partly ticked.
        let suggested = r.ticked.clone();
        assert_eq!(r.tick_all(), Tick::Some);
        r.toggle_all();
        assert_eq!((r.tick_all(), r.count()), (Tick::None, 0));
        assert_eq!(r.move_state(), MoveState::NothingSelected);
        r.toggle_all();
        assert_eq!(r.ticked, suggested, "back to the suggested ones");
    }

    #[test]
    fn the_move_button_says_what_it_waits_for() {
        let mut r = large();
        r.key = Some(PhoneKey::default());
        assert_eq!(r.move_state(), MoveState::EnterKey);
        r.key = Some(PhoneKey {
            typed: "4821-9930-1274".to_owned(),
            checking: false,
        });
        assert_eq!(r.move_state(), MoveState::CheckKey);
        r.key.as_mut().unwrap().checking = true;
        assert_eq!(r.move_state(), MoveState::CheckingKey);
        r.key = None;
        r.listing = true;
        assert_eq!(r.move_state(), MoveState::CheckingVault);
        r.listing = false;
        assert_eq!(r.move_state(), MoveState::Ready(r.count()));
    }

    #[test]
    fn names_say_an_unchecked_vault_once_and_ask_only_about_clashes() {
        let mut r = large();
        let plan = r.plan("Dana's MacBook", "machine-danas-macbook-0000");
        let n = Naming::of(&plan);
        // Not checked: no choice per project, one condition, every project with its suffix apart.
        assert!(n.decisions.is_empty());
        assert_eq!(n.unchecked.as_ref().unwrap().why, KEY_UNCHECKED);
        assert!(n.projects.len() >= 20, "{}", n.projects.len());
        assert!(n.projects.iter().all(|p| p.how == How::Suffixed && p.after.len() == 5 && p.after.starts_with('-')));
        let voice = n.projects.iter().find(|p| p.base == "dana-voice").unwrap();
        assert_eq!(voice.name(), format!("dana-voice{}", voice.after));
        // The same variable in several files of one project: grouped by situation.
        assert!(
            n.duplicates.len() < plan.conflicts.len(),
            "grouped: {} < {}",
            n.duplicates.len(),
            plan.conflicts.len()
        );
        assert_eq!(n.duplicate_names(), plan.conflicts.len());
        assert!(
            n.duplicates.iter().any(|g| g.kept == "voice/serv/.env" && g.names.contains(&"DISCORD_TOKEN".to_owned()))
        );
        assert!(n.machine.1 > 0 && n.ssh == 1 && n.tokens == 2);
        // Checked: the three of the same names from elsewhere are choices; the rest have their own names.
        r.existing = Demo::large_vault();
        let plan = r.plan("Dana's MacBook", "machine-danas-macbook-0000");
        let n = Naming::of(&plan);
        assert!(n.unchecked.is_none());
        let mut asked: Vec<&str> = n.decisions.iter().map(|c| c.name.as_str()).collect();
        asked.sort_unstable();
        assert_eq!(asked, ["api", "dana-blog", "dana-voice"]);
        let api = n.projects.iter().find(|p| p.base == "api").unwrap();
        assert_eq!((api.before.as_str(), api.how), ("dev-", How::Renamed));
        assert!(n.projects.iter().any(|p| p.name() == "acme-web" && p.how == How::AsIs && p.envs == 2));
        r.set_merge("api", true);
        let n = Naming::of(&r.plan("Dana's MacBook", "machine-danas-macbook-0000"));
        assert!(n.projects.iter().any(|p| p.name() == "api" && p.how == How::Merged));
    }

    #[test]
    fn the_move_stops_when_the_vault_changed_the_names() {
        let mut r = large();
        let before = r.plan("Dana's MacBook", "machine-x-0000");
        assert!(same_places(&before, &r.plan("Dana's MacBook", "machine-x-0000")));
        r.existing = Demo::large_vault();
        assert!(!same_places(&before, &r.plan("Dana's MacBook", "machine-x-0000")));
    }

    #[test]
    fn the_phone_key_is_twelve_digits_however_they_are_typed() {
        let key = |t: &str| PhoneKey {
            typed: t.to_owned(),
            checking: false,
        };
        for typed in ["4821-9930-1274", "4821 9930 1274", "482199301274"] {
            assert_eq!(key(typed).digits().as_deref(), Some("482199301274"), "{typed}");
        }
        assert_eq!(key("4821-9930-127").digits(), None);
        assert_eq!(key("").digits(), None);
    }
}
