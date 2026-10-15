//! Step 3 of import: each source whose selected secrets all reached the vault stops holding them (design
//! `docs/secrets.md`, 5.3). A file is never changed in place: the original goes to the trash first, then the new
//! one is written, and the receipt says so ([`Change::Replaced`]), so undo puts the original back.
//!
//! Each file is edited as it was read for the move (the same text the values were sent from), only while it still
//! holds exactly that text, and only when what would stay holds none of the moved values. The originals in the trash
//! are still plain text: they count as on disk until shredded ([`super::receipt::purge`]).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use zeroize::Zeroizing;

use super::{Change, DEFAULT_ENV, Done, Machine, PendingKey, Plan, Receipt, Texts, env_for, var_name};
use crate::project::{
    Confirm, ENV_FILE, EnvFile, EnvSettings, EnvValue, PROJECT_FILE, ProjectFile, parse_dotenv_spans,
};
use crate::secrets::{EnvRef, Reference};
use crate::secrets_scan::sources::{self, RawEntry};
use crate::secrets_scan::{self, Confidence, Finding, ScanReport, Source};

/// How long a release of an environment may be held, as import writes it: a day's work, or a quarter of an hour for
/// production (which is asked every time too).
const LEASE_SECS: u32 = 8 * 3_600;
const PRODUCTION_LEASE_SECS: u32 = 15 * 60;

/// `reins.toml` for a project import made: its name and origin, and its environments' settings.
fn project_file(project: &str, origin: Option<&str>, envs: &[&super::EnvTarget]) -> Result<ProjectFile, String> {
    let mut file = ProjectFile::new(project, DEFAULT_ENV)?;
    file.origin = origin.map(str::to_owned);
    for e in envs {
        let production = matches!(e.env.as_str(), "production" | "prod");
        file.envs.insert(
            e.env.clone(),
            EnvSettings {
                lease_secs: Some(if production {
                    PRODUCTION_LEASE_SECS
                } else {
                    LEASE_SECS
                }),
                confirm: if production {
                    Confirm::Always
                } else {
                    Confirm::Default
                },
            },
        );
    }
    Ok(file)
}

/// Most variables in one run profile (`reins run`'s limit for one release).
const PROFILE_MAX: usize = crate::secrets::MAX_SECRETS;

/// The mode a file gets: `public` files (reins.toml, .env.reins: references only) 0644; a replacement keeps its
/// original's, and at most 0600 when the original was private.
fn mode_for(original: Option<&Path>) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        match original.and_then(|p| std::fs::metadata(p).ok()) {
            Some(m) => {
                let mode = m.permissions().mode() & 0o777;
                // Nothing for the group or others (the low six bits).
                let private = mode.trailing_zeros() >= 6;
                if private {
                    mode & 0o600
                } else {
                    mode
                }
            }
            None if original.is_some() => 0o600,
            None => 0o644,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = original;
        0o600
    }
}

/// A temporary file next to the one it will become: removed when dropped, unless it took that file's place.
struct Temp {
    path: Option<PathBuf>,
}

impl Temp {
    /// Writes `text` to a new file next to `path`, 0600 from its creation (then `mode`).
    fn write(path: &Path, text: &str, mode: u32) -> Result<Self, String> {
        use std::io::Write as _;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let at = path.with_file_name(format!(".{name}.reins-{}", crate::server::random_token(6)));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&at).map_err(|e| format!("{}: {e}", path.display()))?;
        // From here on, every way out removes it.
        let temp = Self {
            path: Some(at),
        };
        let written = file.write_all(text.as_bytes()).and_then(|()| file.sync_all());
        drop(file);
        #[cfg(unix)]
        let written = written.and_then(|()| {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(mode))
        });
        #[cfg(not(unix))]
        let _ = mode;
        written.map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(temp)
    }

    fn path(&self) -> &Path {
        self.path.as_deref().unwrap_or_else(|| Path::new(""))
    }

    /// Takes `to`'s place.
    fn persist(mut self, to: &Path) -> Result<(), String> {
        std::fs::rename(self.path(), to).map_err(|e| format!("{}: {e}", to.display()))?;
        self.path = None;
        Ok(())
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        if let Some(p) = self.path.take() {
            std::fs::remove_file(p).ok();
        }
    }
}

/// Writes a new file (nothing may be there), through a temporary file.
fn create(path: &Path, text: &str, receipt: &mut Receipt) -> Result<(), String> {
    let tmp = Temp::write(path, text, mode_for(None))?;
    if std::fs::symlink_metadata(path).is_ok() {
        return Err(format!("{}: a file is there; left alone", path.display()));
    }
    tmp.persist(path)?;
    receipt.changes.push(Change::Created {
        path: path.to_path_buf(),
    });
    Ok(())
}

/// Writes `text` in the place of `path`: the new file is made first (with the original's permissions), then the
/// original goes to the trash and the new one takes its place. When that last step fails, the original comes back.
fn replace(path: &Path, text: &str, machine: &dyn Machine, receipt: &mut Receipt) -> Result<(), String> {
    let tmp = Temp::write(path, text, mode_for(Some(path)))?;
    let trash_id = machine.trash(path)?;
    if let Err(e) = tmp.persist(path) {
        return match machine.restore(path, trash_id.as_deref()) {
            Ok(()) => Err(e),
            Err(back) => {
                // The original is in the trash, and the receipt says so: undo brings it back.
                receipt.changes.push(Change::Trashed {
                    path: path.to_path_buf(),
                    trash_id,
                });
                Err(format!("{e}; the original is in the trash ({back})"))
            }
        };
    }
    receipt.changes.push(Change::Replaced {
        path: path.to_path_buf(),
        trash_id,
    });
    Ok(())
}

/// Whether `text` still holds one of `values` (non-empty ones): as written, escaped as in double quotes, or (for a
/// value of several lines) one of its longer lines.
fn holds_any<'a>(text: &str, values: impl IntoIterator<Item = &'a str>) -> bool {
    values.into_iter().filter(|v| !v.is_empty()).any(|v| {
        let escaped =
            v.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\r', "\\r").replace('\t', "\\t");
        text.contains(v)
            || text.contains(&escaped)
            || (v.contains('\n') && v.lines().map(str::trim).filter(|l| l.len() >= 8).any(|l| text.contains(l)))
    })
}

/// `text` without the bytes of `spans` (sorted, apart); a byte order mark at the start stays.
fn without_spans(text: &str, spans: &[std::ops::Range<usize>]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for s in spans {
        let start = if s.start == 0 && text.starts_with('\u{feff}') {
            '\u{feff}'.len_utf8()
        } else {
            s.start
        };
        if start < at || s.end > text.len() {
            continue;
        }
        out.push_str(&text[at..start]);
        at = s.end;
    }
    out.push_str(&text[at..]);
    out
}

/// Whether the file at `path` still holds exactly `text` (what was read for the move).
fn unchanged(path: &Path, text: &str) -> bool {
    std::fs::read(path).is_ok_and(|now| now == text.as_bytes())
}

fn trash(path: &Path, machine: &dyn Machine, receipt: &mut Receipt) -> Result<(), String> {
    let trash_id = machine.trash(path)?;
    receipt.changes.push(Change::Trashed {
        path: path.to_path_buf(),
        trash_id,
    });
    Ok(())
}

/// `text` with lines changed: by 1-based number, `None` drops the line, `Some` replaces it (keeping its line ending).
fn edit_lines(text: &str, edits: &BTreeMap<u32, Option<String>>) -> String {
    let mut out = String::with_capacity(text.len());
    for (n, line) in text.split_inclusive('\n').enumerate() {
        match edits.get(&u32::try_from(n + 1).unwrap_or(u32::MAX)) {
            Some(None) => {}
            Some(Some(new)) => {
                let body = line.trim_end_matches(['\n', '\r']);
                out.push_str(new);
                out.push_str(&line[body.len()..]);
            }
            None => out.push_str(line),
        }
    }
    out
}

fn note(receipt: &mut Receipt, r: Result<(), String>) {
    if let Err(e) = r {
        receipt.notes.push(e);
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// What the step took off the disk.
#[derive(Debug, Default)]
pub(super) struct Gone {
    /// The items no longer in their files.
    pub off: BTreeSet<String>,
    /// Of those, the ones whose original file is in the trash: still plain text there until it is shredded or the
    /// trash is emptied.
    pub trashed: BTreeSet<String>,
}

impl Gone {
    /// `items` left their file, whose original went to the trash.
    fn trashed<'a>(&mut self, items: impl IntoIterator<Item = &'a String>) {
        for i in items {
            self.off.insert(i.clone());
            self.trashed.insert(i.clone());
        }
    }
}

/// The finding's text as read for the move, if the file still holds exactly it; else a note says it was left alone.
fn same_text<'t>(f: &Finding, texts: &'t Texts, receipt: &mut Receipt) -> Option<&'t str> {
    match texts.get(&f.id) {
        Some(text) if unchanged(&f.abs_path, text) => Some(text.as_str()),
        _ => {
            receipt.notes.push(format!(
                "{} was left as it is: it changed while its secrets were moving. Your vault has them; scan again.",
                f.path
            ));
            None
        }
    }
}

/// Runs the step for every finding whose selected items all arrived, on the text each file was read as for the move
/// (`texts`), and only while the file still holds it.
pub(super) async fn all(
    report: &ScanReport,
    selected: &BTreeSet<String>,
    arrived: &BTreeSet<String>,
    plan: &Plan,
    texts: &Texts,
    machine: &dyn Machine,
    receipt: &mut Receipt,
) -> Gone {
    let mut ready: Vec<&Finding> = Vec::new();
    for f in &report.findings {
        let chosen: Vec<&str> = f.items.iter().filter(|i| selected.contains(&i.id)).map(|i| i.id.as_str()).collect();
        if chosen.is_empty() || !chosen.iter().any(|i| arrived.contains(*i)) {
            continue;
        }
        if chosen.iter().all(|i| arrived.contains(*i)) {
            ready.push(f);
        } else {
            receipt.notes.push(format!("{} was left as it is: not all of its secrets reached your vault.", f.path));
        }
    }
    let mut gone = Gone::default();
    dotenv(&ready, selected, plan, texts, machine, receipt, &mut gone);
    ssh(&ready, texts, machine, receipt, &mut gone).await;
    git(&ready, selected, plan, texts, machine, receipt, &mut gone);
    machine_files(&ready, selected, plan, texts, machine, receipt, &mut gone);
    gone
}

/// One `.env` file's rewrite: what leaves it, and what stays.
struct DotenvEdit {
    /// The new text (`None`: nothing may stay; the file goes to the trash).
    text: Option<String>,
    /// The values moved to the vault from it (to check that neither file holds them).
    moved: Vec<Zeroizing<String>>,
    /// Variables not moved that stay, and lines Reins cannot read (they stay as written).
    kept: usize,
    unreadable: usize,
}

/// `.env` files: per project, `reins.toml` (when missing) and `.env.reins` (the development variables' references
/// and its plain settings); each file goes to the trash, or keeps only what was not moved and may be a secret.
///
/// What leaves a file is exactly the bytes [`crate::project::parse_dotenv_spans`] read the moved values from, in the
/// same text the values were read from; the new text and `.env.reins` are checked not to hold any moved value before
/// either is written.
fn dotenv(
    ready: &[&Finding],
    selected: &BTreeSet<String>,
    plan: &Plan,
    texts: &Texts,
    machine: &dyn Machine,
    receipt: &mut Receipt,
    gone: &mut Gone,
) {
    let mut by_root: BTreeMap<PathBuf, Vec<&Finding>> = BTreeMap::new();
    for f in ready.iter().filter(|f| f.source == Source::Dotenv) {
        let root = f.project_root.clone().or_else(|| f.abs_path.parent().map(Path::to_path_buf)).unwrap_or_default();
        by_root.entry(root).or_default().push(f);
    }
    let mut files = 0;
    let mut in_history = Vec::new();
    for (root, mut found) in by_root {
        // The less specific file first: the more specific one's line wins in `.env.reins`, as in the vault.
        found.sort_by_key(|f| {
            (f.abs_path.file_name().map(|n| super::dotenv_rank(&n.to_string_lossy())), f.abs_path.clone())
        });
        let envs: Vec<&super::EnvTarget> =
            plan.envs.iter().filter(|e| e.project_root.as_deref() == Some(root.as_path())).collect();
        let Some((project, origin)) = envs.first().map(|e| (e.project.clone(), e.origin.clone())) else {
            continue;
        };
        let toml = root.join(PROJECT_FILE);
        if !toml.exists() {
            let r = project_file(&project, origin.as_deref(), &envs)
                .and_then(|file| create(&toml, &file.render(), receipt));
            note(receipt, r);
        }
        // `.env.reins`: the one there (its other lines kept), else a new one.
        let reins_path = root.join(ENV_FILE);
        let mut env_file = if reins_path.exists() {
            match EnvFile::load(&reins_path) {
                Ok(file) => Some(file),
                Err(e) => {
                    receipt.notes.push(format!("{ENV_FILE} was left as it is: {e}"));
                    None
                }
            }
        } else {
            Some(EnvFile::new())
        };
        let mut env_changed = false;
        // Each file's edit, worked out before anything is written.
        let mut edits: Vec<(&Finding, DotenvEdit)> = Vec::new();
        for f in found {
            if f.git_commit.is_some() {
                in_history.push(f.title.clone());
            }
            let file_name = f.abs_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let env = env_for(&file_name);
            let Ok(env_ref) = EnvRef::new(&project, &env) else {
                receipt.notes.push(format!("{} was left as it is: {project}/{env} is not a name", f.path));
                continue;
            };
            let Some(text) = same_text(f, texts, receipt) else {
                continue;
            };
            let parse = parse_dotenv_spans(text);
            let entries = secrets_scan::parse_entries(Source::Dotenv, text);
            let ids = secrets_scan::item_ids(&f.id, &entries);
            // The bytes that leave the file (moved, or now in `.env.reins`); the rest stays as it is written.
            let mut spans = Vec::new();
            let mut edit = DotenvEdit {
                text: None,
                moved: Vec::new(),
                kept: 0,
                unreadable: parse.errors.len(),
            };
            for (var, id) in parse.vars.iter().zip(&ids) {
                let item = f.items.iter().find(|i| &i.id == id);
                let moved = selected.contains(id);
                let secretish = item.is_some_and(|i| i.confidence > Confidence::None);
                let leaves = if moved {
                    // Only the default environment's file gets references: `reins run -e <env>` reads the rest.
                    if env == DEFAULT_ENV
                        && let Some(file) = env_file.as_mut()
                    {
                        let reference = Reference::Field(env_ref.var(&var_name(&var.name)).secret_ref());
                        note(receipt, file.set(&var.name, EnvValue::Reference(reference)));
                        env_changed = true;
                    }
                    edit.moved.push(var.value.clone());
                    true
                } else if !secretish && env == DEFAULT_ENV && env_file.is_some() {
                    // A setting goes to `.env.reins` as it is, when that file can hold it.
                    let set =
                        env_file.as_mut().map(|file| file.set(&var.name, EnvValue::Literal(var.value.to_string())));
                    if set == Some(Ok(())) {
                        env_changed = true;
                        true
                    } else {
                        receipt.notes.push(format!(
                            "{} stays in {}: {ENV_FILE} cannot hold its value as it is",
                            var.name, f.path
                        ));
                        false
                    }
                } else {
                    false
                };
                if leaves {
                    spans.push(var.span.clone());
                } else {
                    edit.kept += 1;
                }
            }
            if edit.kept > 0 {
                receipt.notes.push(format!(
                    "{}: {} not moved stay in it.",
                    f.path,
                    plural(edit.kept, "variable", "variables")
                ));
            }
            if edit.unreadable > 0 {
                let at: Vec<String> = parse.errors.iter().take(5).map(|(n, _)| n.to_string()).collect();
                receipt.notes.push(format!(
                    "{}: {} Reins cannot read stay in it as written (line {}); check them by hand.",
                    f.path,
                    plural(edit.unreadable, "line", "lines"),
                    at.join(", ")
                ));
            }
            if edit.kept + edit.unreadable > 0 {
                edit.text = Some(without_spans(text, &spans));
            }
            edits.push((f, edit));
        }
        let moved: Vec<Zeroizing<String>> = edits.iter().flat_map(|(_, e)| e.moved.iter().cloned()).collect();
        let moved = || moved.iter().map(|v| v.as_str());
        // `.env.reins` first: when it cannot be written (or would hold a moved value), the `.env` files stay.
        if env_changed && let Some(file) = env_file {
            let text = file.render();
            let r = if holds_any(&text, moved()) {
                Err(format!("{ENV_FILE} was not written: it would hold a value that moved to your vault"))
            } else if reins_path.exists() {
                replace(&reins_path, &text, machine, receipt)
            } else {
                create(&reins_path, &text, receipt)
            };
            if let Err(e) = r {
                receipt.notes.push(e);
                for (f, _) in &edits {
                    receipt.notes.push(format!("{} was left as it is: {ENV_FILE} was not written.", f.path));
                }
                continue;
            }
        }
        for (f, edit) in edits {
            let r = match &edit.text {
                None => trash(&f.abs_path, machine, receipt),
                Some(text) if holds_any(text, moved()) => Err(format!(
                    "{} was left as it is: what would stay in it still holds a value that moved to your vault.",
                    f.path
                )),
                Some(text) => replace(&f.abs_path, text, machine, receipt),
            };
            if r.is_ok() {
                files += 1;
                gone.trashed(f.items.iter().filter(|i| selected.contains(&i.id)).map(|i| &i.id));
            }
            note(receipt, r);
        }
    }
    if files > 0 {
        receipt.done.push(Done {
            source: Source::Dotenv,
            what: format!("{} → .env.reins", plural(files, ".env file", ".env files")),
            how: "reins run -- <your command>".to_owned(),
            ok: true,
        });
    }
    for title in in_history {
        receipt.notes.push(format!(
            "{title} is in git history: moving it does not unpublish it. Rotate its keys (make new ones at each service)."
        ));
    }
}

/// SSH keys: ssh asks the Reins agent; once the agent signed with the moved key, the private key file goes to the
/// trash (the `.pub` stays).
async fn ssh(ready: &[&Finding], texts: &Texts, machine: &dyn Machine, receipt: &mut Receipt, gone: &mut Gone) {
    let keys: Vec<&&Finding> = ready.iter().filter(|f| f.source == Source::SshKey).collect();
    if keys.is_empty() {
        return;
    }
    match machine.ssh_setup() {
        Ok(true) => receipt.changes.push(Change::SshSetup),
        Ok(false) => {}
        Err(e) => receipt.notes.push(format!("ssh was not pointed at the Reins agent: {e}")),
    }
    for f in keys {
        let Some(text) = same_text(f, texts, receipt) else {
            continue;
        };
        let public = sources::ssh_key(text).and_then(|k| k.public);
        let Some(public) = public else {
            receipt.notes.push(format!(
                "{} stays: its public key is not in the file, so the agent's signature cannot be checked.",
                f.path
            ));
            continue;
        };
        let (ok, how) = match machine.agent_signs_with(&public).await {
            Ok(()) => match trash(&f.abs_path, machine, receipt) {
                Ok(()) => {
                    gone.trashed(f.items.iter().map(|i| &i.id));
                    (true, "signs on your phone; the file is in the trash".to_owned())
                }
                Err(e) => (false, e),
            },
            Err(why) => {
                receipt.pending_keys.push(PendingKey {
                    path: f.abs_path.clone(),
                    public_key: data_encoding::BASE64.encode(&public),
                });
                (false, format!("kept until the agent signs with it ({why}): reins secrets import --finish"))
            }
        };
        receipt.done.push(Done {
            source: Source::SshKey,
            what: format!("{} → the Reins SSH agent", f.title),
            how,
            ok,
        });
    }
}

/// GitHub CLI and git credentials: git for the host goes through Reins, and the token leaves the file (or the
/// keychain: `gh auth logout`).
fn git(
    ready: &[&Finding],
    selected: &BTreeSet<String>,
    plan: &Plan,
    texts: &Texts,
    machine: &dyn Machine,
    receipt: &mut Receipt,
    gone: &mut Gone,
) {
    let found: Vec<&&Finding> =
        ready.iter().filter(|f| matches!(f.source, Source::GhCli | Source::GitCredentials)).collect();
    if found.is_empty() {
        return;
    }
    let hosts: BTreeSet<&str> = plan
        .tokens
        .iter()
        .filter(|t| found.iter().any(|f| f.id == t.finding || t.covers.iter().any(|c| c.starts_with(&f.id))))
        .map(|t| t.host.as_str())
        .collect();
    for host in &hosts {
        match machine.git_through_reins(host) {
            Ok(()) => receipt.changes.push(Change::GitHost {
                host: (*host).to_owned(),
            }),
            Err(e) => receipt.notes.push(format!("git for {host} does not go through Reins yet: {e}")),
        }
    }
    let mut ok = true;
    for f in &found {
        let mut in_file: Vec<&String> = Vec::new();
        for item in f.items.iter().filter(|i| selected.contains(&i.id)) {
            if item.id.ends_with("/keychain") {
                let host = item.group.clone().unwrap_or_default();
                match machine.gh_logout(&host) {
                    Ok(()) => {
                        gone.off.insert(item.id.clone());
                        receipt.changes.push(Change::GhLogout {
                            host,
                        });
                    }
                    Err(e) => {
                        ok = false;
                        receipt.notes.push(format!("the GitHub CLI is still signed in to {host}: {e}"));
                    }
                }
            } else {
                in_file.push(&item.id);
            }
        }
        if !in_file.is_empty() {
            let Some(text) = same_text(f, texts, receipt) else {
                ok = false;
                continue;
            };
            let r = rewrite_lines(f, text, &in_file, |_| None, machine, receipt);
            if r.is_ok() {
                gone.trashed(in_file);
            }
            ok &= r.is_ok();
            note(receipt, r);
        }
    }
    receipt.done.push(Done {
        source: Source::GhCli,
        what: format!("gh and git → git through Reins ({})", hosts.into_iter().collect::<Vec<_>>().join(", ")),
        how: "pushes ask your phone; no token on disk".to_owned(),
        ok,
    });
}

/// The computer's own credentials: run profiles for the tools that read variables (AWS, npm, PyPI, cargo, shell
/// exports) and the token out of their files; the tools that read only their own file keep it.
fn machine_files(
    ready: &[&Finding],
    selected: &BTreeSet<String>,
    plan: &Plan,
    texts: &Texts,
    machine: &dyn Machine,
    receipt: &mut Receipt,
    gone: &mut Gone,
) {
    for f in ready {
        let profiles = match f.source {
            Source::Aws | Source::Npm | Source::PyPi | Source::Cargo | Source::Shell => {
                add_profiles(f, plan, machine, receipt)
            }
            Source::Docker | Source::Gcp | Source::Kube | Source::Netrc => {
                receipt.done.push(Done {
                    source: f.source,
                    what: format!("{} → your vault", f.title),
                    how: format!("still on disk in {}: the tool reads only that file (your vault has a copy)", f.path),
                    ok: false,
                });
                continue;
            }
            Source::Dotenv | Source::SshKey | Source::GhCli | Source::GitCredentials => continue,
        };
        let Some(profiles) = profiles else {
            continue;
        };
        let Some(text) = same_text(f, texts, receipt) else {
            continue;
        };
        let lines: Vec<&str> = text.lines().collect();
        let moving: Vec<&String> = f.items.iter().filter(|i| selected.contains(&i.id)).map(|i| &i.id).collect();
        let r = rewrite_lines(
            f,
            text,
            &moving,
            |e| match f.source {
                Source::Npm => {
                    let (_, var) = super::machine_var(f.source, e.group.as_deref(), &e.name);
                    lines
                        .get((e.line as usize).saturating_sub(1))
                        .and_then(|l| l.split_once('='))
                        .map(|(key, _)| format!("{}=${{{var}}}", key.trim_end()))
                }
                Source::Shell => Some(format!(
                    "# {} is in your vault now (Reins): reins run --profile {} -- <command>",
                    e.name,
                    profiles.first().map_or("shell", String::as_str)
                )),
                _ => None,
            },
            machine,
            receipt,
        );
        let ok = r.is_ok();
        if ok {
            gone.trashed(moving);
        }
        note(receipt, r);
        let how = match f.source {
            Source::Npm => format!("reins run --profile {} -- npm publish", profiles[0]),
            Source::PyPi => format!("reins run --profile {} -- twine upload dist/*", profiles[0]),
            Source::Cargo => format!("reins run --profile {} -- cargo publish", profiles[0]),
            _ => format!("reins run --profile {} -- <command>", profiles.last().map_or("", String::as_str)),
        };
        let names: Vec<&str> = profiles.iter().map(String::as_str).collect();
        receipt.done.push(Done {
            source: f.source,
            what: format!(
                "{} → run {} {}",
                f.source.label().split_whitespace().next().unwrap_or_default(),
                if names.len() == 1 {
                    "profile"
                } else {
                    "profiles"
                },
                names.join(", ")
            ),
            how,
            ok,
        });
    }
}

/// Rewrites `f`'s file, read as `text`: the lines of the entries `ids` are dropped, or replaced by `line(entry)`.
/// Refused (nothing written) when an entry is not where the text has it, or when what stays still holds one of
/// their values.
fn rewrite_lines(
    f: &Finding,
    text: &str,
    ids: &[&String],
    line: impl Fn(&RawEntry) -> Option<String>,
    machine: &dyn Machine,
    receipt: &mut Receipt,
) -> Result<(), String> {
    let entries = secrets_scan::parse_entries(f.source, text);
    let all = secrets_scan::item_ids(&f.id, &entries);
    let mut edits: BTreeMap<u32, Option<String>> = BTreeMap::new();
    let mut values: Vec<&str> = Vec::new();
    for (e, id) in entries.iter().zip(&all) {
        if !ids.contains(&id) {
            continue;
        }
        values.push(&e.value);
        for n in e.line..e.line.saturating_add(e.lines.max(1)) {
            edits.insert(n, None);
        }
        if let Some(new) = line(e) {
            edits.insert(e.line, Some(new));
        }
    }
    if values.len() != ids.len() {
        return Err(format!("{} was left as it is: it does not read as it did at the scan.", f.path));
    }
    let new = edit_lines(text, &edits);
    if holds_any(&new, values) {
        return Err(format!(
            "{} was left as it is: what would stay in it still holds a value that moved to your vault.",
            f.path
        ));
    }
    replace(&f.abs_path, &new, machine, receipt)
}

/// `[run.profiles.<env>]` for the machine environments of `f`'s moved variables; their names.
fn add_profiles(f: &Finding, plan: &Plan, machine: &dyn Machine, receipt: &mut Receipt) -> Option<Vec<String>> {
    let mut names = Vec::new();
    for env in plan.envs.iter().filter(|e| e.project_root.is_none()) {
        let vars: Vec<&super::PlannedVar> = env.vars.iter().filter(|v| v.finding == f.id).collect();
        for (n, chunk) in vars.chunks(PROFILE_MAX).enumerate() {
            let name = if n == 0 {
                env.env.clone()
            } else {
                format!("{}-{}", env.env, n + 1)
            };
            let map: BTreeMap<String, String> =
                chunk.iter().map(|v| (v.name.clone(), env.reference(&v.name))).collect();
            match machine.add_profile(&name, &map, &format!("{} from {}", env.env, f.path)) {
                Ok(()) => {
                    receipt.changes.push(Change::Profile {
                        name: name.clone(),
                    });
                    names.push(name);
                }
                Err(e) => {
                    receipt
                        .notes
                        .push(format!("{} stays as it is: the run profile {name} was not added ({e})", f.path));
                    return None;
                }
            }
        }
    }
    (!names.is_empty()).then_some(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_dropped_or_replaced_by_number() {
        let edits: BTreeMap<u32, Option<String>> = [(2, None), (3, Some("x=${T}".to_owned()))].into_iter().collect();
        assert_eq!(edit_lines("a\nb\nc\nd\n", &edits), "a\nx=${T}\nd\n");
        assert_eq!(edit_lines("a\nb", &BTreeMap::from([(1, None)])), "b");
        // Line endings stay as they were, a replaced line's too.
        let crlf: BTreeMap<u32, Option<String>> = [(1, None), (2, Some("x".to_owned()))].into_iter().collect();
        assert_eq!(edit_lines("a\r\nb\r\nc", &crlf), "x\r\nc");
    }

    #[test]
    fn spans_leave_exactly_and_a_byte_order_mark_stays() {
        let text = "\u{feff}A=1\nB=2\r\nC=3";
        let parse = parse_dotenv_spans(text);
        let span = |n: &str| parse.vars.iter().find(|v| v.name == n).unwrap().span.clone();
        assert_eq!(without_spans(text, &[span("A")]), "\u{feff}B=2\r\nC=3");
        assert_eq!(without_spans(text, &[span("B"), span("C")]), "\u{feff}A=1\n");
        assert_eq!(without_spans(text, &[span("A"), span("B"), span("C")]), "\u{feff}");
    }

    #[test]
    fn a_moved_value_is_found_as_written_escaped_or_by_its_lines() {
        let v = "pa\"ss\\word\nsecond-line-of-it";
        assert!(holds_any(&format!("X={v}"), [v]));
        assert!(holds_any("X=\"pa\\\"ss\\\\word\\nsecond-line-of-it\"", [v]), "escaped as in double quotes");
        assert!(holds_any("X='second-line-of-it'", [v]), "one of its lines");
        assert!(!holds_any("X=other", [v]));
        assert!(!holds_any("anything", [""]), "an empty value is nothing to find");
    }

    #[test]
    fn a_temporary_file_is_private_and_never_left_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        let tmp = Temp::write(&path, "A=1\n", 0o600).unwrap();
        let at = tmp.path().to_path_buf();
        assert_eq!(at.parent(), Some(dir.path()), "next to the file it becomes");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(std::fs::metadata(&at).unwrap().permissions().mode() & 0o777, 0o600);
        }
        drop(tmp);
        assert!(!at.exists(), "removed when not used");
        // A file in the way: refused, and the temporary file is gone too.
        std::fs::write(&path, "B=2\n").unwrap();
        let mut receipt = Receipt::default();
        assert!(create(&path, "A=1\n", &mut receipt).is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        assert!(receipt.changes.is_empty());
    }
}
