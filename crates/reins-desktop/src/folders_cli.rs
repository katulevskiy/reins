//! `reins folders`: this computer's work folders and scratch folders (`[workspace]` in config.toml), from the command
//! line.
//!
//! - `reins folders` lists them, and says of each whether it counts: built in, signed by the phone, waiting for the
//!   owner to confirm it on the phone, refused. It also lists the scratch folders Reins suggests ([`crate::suggest`]),
//!   each with the command that accepts it.
//! - `reins folders add <path>` adds a work folder (a root: its subfolders are projects), `--scratch` a scratch folder
//!   (deleting in it loses nothing); `reins folders temp <path>` is `add --scratch`. `reins folders remove <path>`
//!   removes one.
//!
//! What is written is `config.toml`, nothing else: this computer's own configuration, which only ever tightens where
//! the phone's signed policy decides ([`crate::policy::effective`]). A folder added here counts at once only when it
//! lies inside a work folder the phone signed already (strictly inside, for a scratch folder); any other is proposed
//! to the phone ([`crate::folders`]) and counts once the owner confirms it there. So the owner running this command is
//! one of the two ways a folder comes to count, and the phone is the other. An AI agent running it is asked about,
//! like any command Reins has no model of.
//!
//! The daemon reads the configuration again every minute, so nothing needs a restart.

use std::collections::BTreeMap;
use std::path::Path;

use reins_policy::desktop_folders::{Refused, inside, refusal, same};
use reins_policy::desktop_policy::{clamp_roots, clamp_scratch};
use reins_policy::effects::{Area, Workspace, normalize};
use reins_proto::desktop::FolderKind;

use crate::config::{Config, Paths, WorkspaceConfig};
use crate::settings::{self, WorkspaceList};
use crate::suggest::{Suggestion, WINDOW_SECS, Why};

/// `reins folders …`.
#[derive(Clone, Debug, clap::Subcommand)]
pub enum FoldersCmd {
    /// List the work folders and scratch folders of this computer, whether each counts, and what Reins suggests.
    List,
    /// Add a work folder: its subfolders are your projects. With --scratch, a scratch folder instead.
    ///
    /// A work folder inside one your phone signed already counts at once, and so does a scratch folder strictly
    /// inside one. Any other shows on your phone as a card (Settings › Computers › this computer › Folders) and
    /// counts once you confirm it there.
    Add {
        /// The folder: absolute, `~/…`, or relative to where you are.
        path: String,
        /// A scratch folder: deleting in it loses nothing, so agents are not asked about it. `/tmp`, `/var/tmp`,
        /// `/dev/shm` and `~/.cache` always are.
        #[arg(long)]
        scratch: bool,
    },
    /// Set a temp folder for this computer: a scratch folder (the same as `add --scratch`).
    ///
    /// `/tmp`, `/var/tmp`, `/dev/shm` and `~/.cache` are always scratch and need no entry. Use this for a temp folder
    /// of your own, such as the one `$TMPDIR` names or `~/Documents/dev/tmp`.
    Temp {
        /// The folder: absolute, `~/…`, or relative to where you are.
        path: String,
    },
    /// Remove a folder from this computer's work folders or scratch folders.
    Remove {
        /// The folder, as `reins folders` lists it.
        path: String,
    },
}

/// Where the owner confirms and removes folders on the phone.
pub const ON_PHONE: &str = "Settings › Computers › this computer › Folders";
/// The folders that are always scratch.
const BUILT_IN: &str = "/tmp, /var/tmp, /dev/shm, ~/.cache";

/// What the phone signed for this computer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Signed {
    pub roots: Vec<String>,
    pub scratch: Vec<String>,
}

/// How a folder of this computer stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Never proposed nor counted (too broad, keys, Reins' own state).
    Refused(Refused),
    /// No policy the phone signed is held: the folder is only set here.
    Unsigned,
    /// The phone signed it.
    Signed,
    /// It lies inside a work folder the phone signed.
    InsideRoot,
    /// The phone signed it as scratch, and no configuration here names it.
    PhoneOnly,
    /// It doesn't resolve on this computer, so it is not proposed.
    Missing,
    /// Proposed to the phone, not confirmed there yet.
    Waiting,
}

impl Status {
    #[must_use]
    pub fn label(self) -> String {
        match self {
            Self::Refused(why) => format!("refused: {}", why.label()),
            Self::Unsigned => "set here (the phone has not signed a policy for this computer yet)".to_owned(),
            Self::Signed => "counts (your phone signed it)".to_owned(),
            Self::InsideRoot => "counts (inside a work folder your phone signed)".to_owned(),
            Self::PhoneOnly => "counts (confirmed on your phone; not in config.toml here)".to_owned(),
            Self::Missing => "not found on this computer (not proposed to your phone)".to_owned(),
            Self::Waiting => "waiting for you to confirm it on your phone".to_owned(),
        }
    }
}

/// The forms `entry` has here: as written (`~` read as `home`), and what it resolves to when that is known.
fn forms(entry: &str, home: &str, canonical: &BTreeMap<String, String>) -> Vec<String> {
    let mut out = vec![normalize(entry, "/", home)];
    if let Some(c) = canonical.get(entry)
        && !out.contains(c)
    {
        out.push(c.clone());
    }
    out
}

/// How `entry` (a `[workspace]` root or scratch folder, as the configuration has it) stands with what the phone
/// `signed` (`None`: no policy held). `canonical`: what the folders named here and signed resolve to on this
/// computer. It says what a hook decides with: the same clamps ([`clamp_roots`], [`clamp_scratch`]).
#[must_use]
pub fn status(
    kind: FolderKind,
    entry: &str,
    home: &str,
    canonical: &BTreeMap<String, String>,
    signed: Option<&Signed>,
) -> Status {
    let forms = forms(entry, home, canonical);
    if let Some(why) = forms.iter().find_map(|f| refusal(f, home)) {
        return Status::Refused(why);
    }
    let Some(signed) = signed else {
        return Status::Unsigned;
    };
    let local = [entry.to_owned()];
    let (counts, named) = match kind {
        FolderKind::Root => (!clamp_roots(&signed.roots, &local, home, canonical).is_empty(), &signed.roots),
        FolderKind::Scratch => (
            clamp_scratch(&signed.roots, &signed.scratch, &local, home, canonical).contains(&forms[0]),
            &signed.scratch,
        ),
    };
    if counts {
        let itself = named.iter().any(|s| forms.iter().any(|f| same(s, f, home)));
        if itself {
            Status::Signed
        } else {
            Status::InsideRoot
        }
    } else if canonical.contains_key(entry) {
        Status::Waiting
    } else {
        Status::Missing
    }
}

/// One folder in the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub kind: FolderKind,
    /// As shown: `~/…` inside the home folder.
    pub path: String,
    pub status: Status,
}

/// This computer's folders and how each stands: the roots, then the scratch folders, as the configuration has them,
/// then the scratch folders the phone signed that no configuration here names (they count too).
#[must_use]
pub fn rows(
    workspace: &WorkspaceConfig,
    home: &str,
    canonical: &BTreeMap<String, String>,
    signed: Option<&Signed>,
) -> Vec<Row> {
    let row = |kind, entry: &String| Row {
        kind,
        path: shown(&normalize(entry, "/", home), home),
        status: status(kind, entry, home, canonical, signed),
    };
    let mut out: Vec<Row> = workspace.roots.iter().map(|r| row(FolderKind::Root, r)).collect();
    out.extend(workspace.scratch.iter().map(|s| row(FolderKind::Scratch, s)));
    if let Some(signed) = signed {
        let counted = clamp_scratch(&signed.roots, &signed.scratch, &[], home, canonical);
        let local: Vec<String> = workspace.scratch.iter().flat_map(|s| forms(s, home, canonical)).collect();
        for s in counted.iter().filter(|s| !local.iter().any(|l| same(l, s, home))) {
            out.push(Row {
                kind: FolderKind::Scratch,
                path: shown(s, home),
                status: Status::PhoneOnly,
            });
        }
    }
    out
}

/// `path` (absolute) as people write it: `~/…` inside the home folder.
#[must_use]
pub fn shown(path: &str, home: &str) -> String {
    let home = home.trim_end_matches('/');
    match path.strip_prefix(home) {
        Some("") if !home.is_empty() => "~".to_owned(),
        Some(rest) if !home.is_empty() && rest.starts_with('/') => format!("~{rest}"),
        _ => path.to_owned(),
    }
}

/// `path` as it can be pasted into a shell: quoted when it holds anything but plain path characters (`~/` stays
/// outside the quotes, so the shell still reads it as the home folder).
#[must_use]
pub fn quoted(path: &str) -> String {
    let plain = |s: &str| s.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+@%,:=".contains(c));
    let quote = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    match path.strip_prefix("~/") {
        _ if plain(path) || path == "~" => path.to_owned(),
        Some(rest) if plain(rest) => path.to_owned(),
        Some(rest) => format!("~/{}", quote(rest)),
        None => quote(path),
    }
}

/// What the owner typed, as an absolute path: `~` read as `home`, a relative path read from `cwd`, `.` and `..`
/// resolved as written.
pub fn written(input: &str, cwd: &str, home: &str) -> Result<String, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("no folder given".to_owned());
    }
    if input.chars().any(char::is_control) {
        return Err("a folder's name can't hold control characters".to_owned());
    }
    let path = normalize(input, cwd, home);
    if path.starts_with('/') {
        Ok(path)
    } else {
        Err(format!("{input} is not a path Reins can read (folders are `/…` or `~/…` paths)"))
    }
}

/// `path` (absolute) is in a folder that is always scratch.
fn built_in(path: &str, home: &str) -> bool {
    Workspace {
        home: home.trim_end_matches('/').to_owned(),
        ..Workspace::default()
    }
    .area(path)
        == Area::Scratch
}

/// The lines of the list: the folders, what Reins suggests, and what to do next.
#[must_use]
pub fn lines(rows: &[Row], suggested: &[Suggestion], home: &str) -> Vec<String> {
    let width = rows.iter().map(|r| r.path.chars().count()).max().unwrap_or(0).max(BUILT_IN.len());
    let line = |path: &str, status: &str| format!("  {path:<width$}  {status}");
    let mut out = vec!["Work folders (their subfolders are your projects):".to_owned()];
    let roots: Vec<&Row> = rows.iter().filter(|r| r.kind == FolderKind::Root).collect();
    if roots.is_empty() {
        out.push("  none".to_owned());
    }
    out.extend(roots.iter().map(|r| line(&r.path, &r.status.label())));
    out.push(String::new());
    out.push("Scratch folders (deleting in them loses nothing, so it is not asked about):".to_owned());
    out.push(line(BUILT_IN, "built in"));
    out.extend(rows.iter().filter(|r| r.kind == FolderKind::Scratch).map(|r| line(&r.path, &r.status.label())));
    if !suggested.is_empty() {
        out.push(String::new());
        out.push("Suggested scratch folders:".to_owned());
        for s in suggested {
            let path = shown(&s.path, home);
            let why = match s.why {
                Why::Approved(times) => {
                    format!("you approved {times} deletes in it in the last {} days", WINDOW_SECS / 86_400)
                }
                Why::TempDir => "the temp folder here ($TMPDIR)".to_owned(),
            };
            out.push(format!("  {path}  ({why})"));
            out.push(format!("      reins folders add --scratch {}", quoted(&path)));
        }
        out.push(
            "  Nothing suggested counts until you run its command here, or confirm its card on your phone.".to_owned(),
        );
    }
    out.push(String::new());
    if rows.iter().any(|r| r.status == Status::Waiting) {
        out.push(format!("Confirm on your phone: {ON_PHONE}."));
    }
    out.push(
        "`reins folders add <path>` adds a work folder (`--scratch`: a scratch folder), `reins folders remove <path>` \
         removes one."
            .to_owned(),
    );
    out
}

/// Where this command runs.
struct Here {
    /// The home folder, as written (what `~` is for the engine).
    home: String,
    cwd: String,
}

impl Here {
    fn find() -> Result<Self, String> {
        let home = crate::config::home_dir()?;
        let cwd = std::env::current_dir().map_err(|e| format!("cannot tell which folder this is: {e}"))?;
        Ok(Self {
            home: home.to_str().ok_or("the home folder's name is not UTF-8")?.to_owned(),
            cwd: cwd.to_str().unwrap_or_default().to_owned(),
        })
    }
}

/// What the phone signed, when a policy is held, and what every folder named here or signed resolves to.
fn held(paths: &Paths, config: &Config, now: i64) -> (Option<Signed>, BTreeMap<String, String>) {
    let (_, held) = crate::policy::effective(paths, config, now);
    let signed = held.policy.map(|p| Signed {
        roots: p.roots,
        scratch: p.scratch,
    });
    let named = signed.iter().flat_map(|s| s.roots.iter().chain(&s.scratch));
    let canonical =
        crate::folders::resolved(config.workspace.roots.iter().chain(&config.workspace.scratch).chain(named));
    (signed, canonical)
}

/// `reins folders` and `reins folders list`. Without a policy from the phone, the last line says why (as `reins
/// status` does).
pub fn list(paths: &Paths, config: &Config, now: i64) -> Result<Vec<String>, String> {
    let here = Here::find()?;
    let (signed, canonical) = held(paths, config, now);
    let rows = rows(&config.workspace, &here.home, &canonical, signed.as_ref());
    let suggested = crate::suggest::suggestions(paths, config, now);
    let mut out = lines(&rows, &suggested, &here.home);
    if signed.is_none() {
        out.push(if crate::server::oauth::logged_in_server(paths).is_some() {
            format!("Auto rules: {}.", crate::policy::status_line(paths, config, now))
        } else {
            "This computer is not paired with a phone yet (`reins setup`).".to_owned()
        });
    }
    Ok(out)
}

fn what(list: WorkspaceList) -> &'static str {
    match list {
        WorkspaceList::Roots => "work folder",
        WorkspaceList::Scratch => "scratch folder",
    }
}

/// `reins folders add` and `reins folders temp`: checks the folder, writes it to `config.toml`, and says what
/// happens next.
///
/// Refused: what [`refusal`] refuses (as written, and as it resolves here), a folder that is scratch already, a
/// scratch folder that is or holds a work folder (its projects would become scratch), a folder already in the list.
/// A folder that doesn't exist yet is added when the folder it would be in exists (a temp folder is often made
/// later); it is proposed to the phone once it exists.
pub fn add(paths: &Paths, input: &str, list: WorkspaceList, now: i64) -> Result<Vec<String>, String> {
    let here = Here::find()?;
    let config = Config::load(paths).map_err(|e| e.to_string())?;
    let home = here.home.as_str();
    let path = written(input, &here.cwd, home)?;
    let name = shown(&path, home);
    let what = what(list);
    if built_in(&path, home) {
        return Err(format!("{name} is in a folder that is always scratch ({BUILT_IN}): nothing to add"));
    }
    let glob = path.contains(['*', '?', '[']);
    let resolved = crate::folders::canonical(&path, home);
    let checked = match &resolved {
        Some(r) if !glob && !Path::new(r).is_dir() => return Err(format!("{name} is not a folder")),
        Some(r) => r.clone(),
        None if glob => return Err(format!("{name}: the folders before the `*` do not exist")),
        None => {
            let (parent, child) = path.rsplit_once('/').unwrap_or(("", path.as_str()));
            let parent = crate::folders::canonical(
                if parent.is_empty() {
                    "/"
                } else {
                    parent
                },
                home,
            )
            .filter(|p| Path::new(p).is_dir())
            .ok_or_else(|| format!("{name} does not exist, and neither does the folder it would be in"))?;
            format!("{}/{child}", parent.trim_end_matches('/'))
        }
    };
    let resolved_home = crate::folders::home();
    let refused = [(&path, home), (&checked, home), (&checked, resolved_home.as_str())]
        .iter()
        .find_map(|(folder, home)| refusal(folder, home));
    if let Some(why) = refused {
        let on_disk = if checked == path {
            String::new()
        } else {
            format!(" (it is {checked} on this computer)")
        };
        return Err(format!("{name} can't be a {what}: {}{on_disk}", why.label()));
    }
    let is = |entry: &String| {
        normalize(entry, "/", home) == path
            || (resolved.is_some() && crate::folders::canonical(entry, home) == resolved)
    };
    let workspace = &config.workspace;
    let scratch_here = Workspace {
        home: home.to_owned(),
        scratch: workspace.scratch.clone(),
        ..Workspace::default()
    };
    match list {
        WorkspaceList::Roots => {
            if workspace.roots.iter().any(is) {
                return Err(format!("{name} is already a work folder here"));
            }
            if scratch_here.area(&path) == Area::Scratch {
                return Err(format!(
                    "{name} is in a scratch folder of this computer: everything in it is scratch. Remove that one \
                     first (`reins folders` lists them)"
                ));
            }
        }
        WorkspaceList::Scratch => {
            if workspace.scratch.iter().any(is) {
                return Err(format!("{name} is already a scratch folder here"));
            }
            if scratch_here.area(&path) == Area::Scratch {
                return Err(format!("{name} is inside a scratch folder of this computer: it is scratch already"));
            }
            if let Some(root) = workspace.roots.iter().find(|r| inside(&path, r, home)) {
                return Err(format!(
                    "{name} is, or holds, the work folder {root}: the projects in it would count as scratch"
                ));
            }
        }
    }
    settings::add_workspace_folder(paths, list, &name)?;
    let config = Config::load(paths).map_err(|e| e.to_string())?;
    let (signed, canonical) = held(paths, &config, now);
    let kind = match list {
        WorkspaceList::Roots => FolderKind::Root,
        WorkspaceList::Scratch => FolderKind::Scratch,
    };
    let next = match status(kind, &name, home, &canonical, signed.as_ref()) {
        Status::Signed | Status::PhoneOnly => "It counts from now on: your phone signed it already.".to_owned(),
        Status::InsideRoot => "It counts from now on: it is inside a work folder your phone signed.".to_owned(),
        Status::Waiting => format!(
            "It counts once you confirm it on your phone: {ON_PHONE}. It shows there within a minute, while the \
             Reins background service runs."
        ),
        Status::Missing => format!(
            "It does not exist yet. Once it does, it shows on your phone ({ON_PHONE}) and counts once you confirm \
             it there."
        ),
        Status::Unsigned => format!(
            "It is set here. Your phone has not signed a policy for this computer yet; once it has, the folder \
             shows there if it needs confirming ({ON_PHONE})."
        ),
        Status::Refused(why) => format!("It does not count: {}.", why.label()),
    };
    Ok(vec![format!("Added {name} as a {what}."), next])
}

/// `reins folders remove`: takes the folder out of `[workspace]` (whichever list has it), and says whether it still
/// counts (a scratch folder the phone signed does, until it is removed there).
pub fn remove(paths: &Paths, input: &str, now: i64) -> Result<Vec<String>, String> {
    let here = Here::find()?;
    let config = Config::load(paths).map_err(|e| e.to_string())?;
    let home = here.home.as_str();
    let path = written(input, &here.cwd, home)?;
    let name = shown(&path, home);
    let resolved = crate::folders::canonical(&path, home);
    let is = |entry: &&String| {
        entry.as_str() == input.trim()
            || normalize(entry, "/", home) == path
            || (resolved.is_some() && crate::folders::canonical(entry, home) == resolved)
    };
    let roots: Vec<String> = config.workspace.roots.iter().filter(is).cloned().collect();
    let scratch: Vec<String> = config.workspace.scratch.iter().filter(is).cloned().collect();
    let (signed, _) = held(paths, &config, now);
    let signed_scratch = signed.is_some_and(|s| {
        s.scratch.iter().any(|s| same(s, &path, home) || resolved.as_deref().is_some_and(|r| same(s, r, home)))
    });
    if roots.is_empty() && scratch.is_empty() {
        return Err(if built_in(&path, home) {
            format!("{name} is always scratch ({BUILT_IN} are built in): there is nothing to remove")
        } else if signed_scratch {
            format!(
                "{name} is not in this computer's config.toml. Your phone signed it as a scratch folder: remove it \
                 there ({ON_PHONE})"
            )
        } else {
            format!("{name} is not one of this computer's folders (`reins folders` lists them)")
        });
    }
    let mut out = Vec::new();
    for entry in &roots {
        settings::remove_workspace_folder(paths, WorkspaceList::Roots, entry)?;
        out.push(format!("Removed {name} from this computer's work folders."));
    }
    for entry in &scratch {
        settings::remove_workspace_folder(paths, WorkspaceList::Scratch, entry)?;
        out.push(format!("Removed {name} from this computer's scratch folders."));
    }
    out.push(if signed_scratch {
        format!(
            "Your phone still has it signed as a scratch folder, so it still counts: remove it there too ({ON_PHONE})."
        )
    } else {
        "It no longer counts here.".to_owned()
    });
    Ok(out)
}

/// Runs `reins folders …`; the lines to print.
pub fn run(action: Option<&FoldersCmd>, paths: &Paths, config: &Config) -> Result<Vec<String>, String> {
    let now = crate::now_unix();
    match action {
        None | Some(FoldersCmd::List) => list(paths, config, now),
        Some(FoldersCmd::Add {
            path,
            scratch,
        }) => {
            let list = if *scratch {
                WorkspaceList::Scratch
            } else {
                WorkspaceList::Roots
            };
            add(paths, path, list, now)
        }
        Some(FoldersCmd::Temp {
            path,
        }) => add(paths, path, WorkspaceList::Scratch, now),
        Some(FoldersCmd::Remove {
            path,
        }) => remove(paths, path, now),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/home/me";

    fn signed() -> Signed {
        Signed {
            roots: ["~/Documents/dev", "~/src", "~/code", "~/projects", "/mnt/data/dev"].map(str::to_owned).to_vec(),
            scratch: vec!["/home/me/.zeron/uploads".to_owned(), "/home/me/Downloads/tmp".to_owned()],
        }
    }

    /// Every folder resolves to itself, but `~/missing`.
    fn resolving(entries: &[&str]) -> BTreeMap<String, String> {
        entries.iter().filter(|e| !e.contains("missing")).map(|e| ((*e).to_owned(), normalize(e, "/", HOME))).collect()
    }

    #[test]
    fn a_folder_counts_when_the_phone_signed_it_or_a_root_around_it() {
        let s = signed();
        let all = [
            "~/Documents/dev",
            "~/Documents/dev/client",
            "~/work",
            "/mnt/data/dev",
            "~/Documents/dev/tmp",
            "~/.zeron/uploads",
            "~/Pictures/tmp",
            "~/missing",
            "~/Documents/dev/missing",
        ];
        let canonical = resolving(&all);
        let root = |entry: &str| status(FolderKind::Root, entry, HOME, &canonical, Some(&s));
        let scratch = |entry: &str| status(FolderKind::Scratch, entry, HOME, &canonical, Some(&s));
        assert_eq!(root("~/Documents/dev"), Status::Signed);
        assert_eq!(root("/mnt/data/dev"), Status::Signed);
        assert_eq!(root("~/Documents/dev/client"), Status::InsideRoot);
        assert_eq!(root("~/work"), Status::Waiting);
        assert_eq!(root("~/missing"), Status::Missing);
        assert_eq!(scratch("~/Documents/dev/tmp"), Status::InsideRoot);
        assert_eq!(scratch("~/.zeron/uploads"), Status::Signed);
        assert_eq!(scratch("~/Pictures/tmp"), Status::Waiting);
        assert_eq!(scratch("~/missing"), Status::Missing);
        // A root itself is never scratch, and a signed root's parent is no root.
        assert_eq!(scratch("~/Documents/dev"), Status::Waiting);
        // Not there yet, but inside a signed root: it counts as written, as a hook would read it.
        assert_eq!(scratch("~/Documents/dev/missing"), Status::InsideRoot);
        // Refused, whatever is signed.
        for (entry, why) in [
            ("~", Refused::TooBroad),
            ("/", Refused::TooBroad),
            ("~/.ssh", Refused::Sensitive),
            ("~/.config/reins", Refused::Sensitive),
            ("/home", Refused::TooBroad),
        ] {
            assert_eq!(root(entry), Status::Refused(why), "{entry}");
            assert_eq!(scratch(entry), Status::Refused(why), "{entry}");
        }
        // No policy from the phone: set here only (refused ones still say so).
        assert_eq!(status(FolderKind::Root, "~/work", HOME, &canonical, None), Status::Unsigned);
        assert_eq!(status(FolderKind::Scratch, "~/Documents/dev/tmp", HOME, &canonical, None), Status::Unsigned);
        assert_eq!(status(FolderKind::Root, "~", HOME, &canonical, None), Status::Refused(Refused::TooBroad));
    }

    /// A folder whose link an agent re-pointed stands as what it resolves to now.
    #[test]
    fn a_folder_stands_as_it_resolves() {
        let s = signed();
        let canonical = BTreeMap::from([
            // A link to a signed root; a link to the home folder; a scratch folder now pointing out of its root.
            ("~/work".to_owned(), "/mnt/data/dev".to_owned()),
            ("~/everything".to_owned(), "/home/me".to_owned()),
            ("~/Documents/dev/tmp".to_owned(), "/home/me/Pictures".to_owned()),
        ]);
        assert_eq!(status(FolderKind::Root, "~/work", HOME, &canonical, Some(&s)), Status::Signed);
        assert_eq!(
            status(FolderKind::Root, "~/everything", HOME, &canonical, Some(&s)),
            Status::Refused(Refused::TooBroad)
        );
        // As a hook reads it: the name still lies in the root (so it counts as written), what it points to does not.
        assert_eq!(status(FolderKind::Scratch, "~/Documents/dev/tmp", HOME, &canonical, Some(&s)), Status::InsideRoot);
    }

    #[test]
    fn the_rows_list_local_folders_then_what_only_the_phone_has() {
        let workspace = WorkspaceConfig {
            roots: vec!["~/Documents/dev".to_owned(), "~/work".to_owned()],
            scratch: vec!["~/Documents/dev/tmp".to_owned(), "~/.zeron/uploads".to_owned()],
        };
        let s = signed();
        let canonical = resolving(&["~/Documents/dev", "~/work", "~/Documents/dev/tmp", "~/.zeron/uploads"]);
        let got: Vec<(FolderKind, String, Status)> =
            rows(&workspace, HOME, &canonical, Some(&s)).into_iter().map(|r| (r.kind, r.path, r.status)).collect();
        assert_eq!(
            got,
            [
                (FolderKind::Root, "~/Documents/dev".to_owned(), Status::Signed),
                (FolderKind::Root, "~/work".to_owned(), Status::Waiting),
                (FolderKind::Scratch, "~/Documents/dev/tmp".to_owned(), Status::InsideRoot),
                (FolderKind::Scratch, "~/.zeron/uploads".to_owned(), Status::Signed),
                // Confirmed on the phone (a suggestion): no configuration here names it, and it counts.
                (FolderKind::Scratch, "~/Downloads/tmp".to_owned(), Status::PhoneOnly),
            ]
        );
        // Without a policy nothing the phone has is known.
        let unsigned = rows(&workspace, HOME, &canonical, None);
        assert_eq!(unsigned.len(), 4);
        assert!(unsigned.iter().all(|r| r.status == Status::Unsigned));
    }

    #[test]
    fn paths_are_read_and_shown_as_people_write_them() {
        let read = |input: &str| written(input, "/home/me/Documents/dev/app", HOME);
        assert_eq!(read("~/Documents/dev/tmp").unwrap(), "/home/me/Documents/dev/tmp");
        assert_eq!(read("  /data/scratch/ ").unwrap(), "/data/scratch");
        assert_eq!(read("tmp").unwrap(), "/home/me/Documents/dev/app/tmp");
        assert_eq!(read("../tmp").unwrap(), "/home/me/Documents/dev/tmp");
        assert_eq!(read(".").unwrap(), "/home/me/Documents/dev/app");
        assert_eq!(read("~").unwrap(), "/home/me");
        assert!(read("").is_err());
        assert!(read("/data/x\u{1b}[2J").is_err());
        // No working directory known: a relative path can't be read.
        assert!(written("tmp", "", HOME).is_err());
        assert_eq!(shown("/home/me/Documents/dev/tmp", HOME), "~/Documents/dev/tmp");
        assert_eq!(shown("/home/me", HOME), "~");
        assert_eq!(shown("/home/meadow/x", HOME), "/home/meadow/x");
        assert_eq!(shown("/data/scratch", HOME), "/data/scratch");
        assert_eq!(shown("/data/scratch", ""), "/data/scratch");
        assert_eq!(quoted("~/Documents/dev/tmp"), "~/Documents/dev/tmp");
        assert_eq!(quoted("~/My Files/tmp"), "~/'My Files/tmp'");
        assert_eq!(quoted("/data/it's here"), "'/data/it'\\''s here'");
        assert_eq!(quoted("/data/$(reboot)"), "'/data/$(reboot)'");
    }

    #[test]
    fn the_list_says_what_counts_and_how_to_accept_a_suggestion() {
        let rows = [
            Row {
                kind: FolderKind::Root,
                path: "~/Documents/dev".to_owned(),
                status: Status::Signed,
            },
            Row {
                kind: FolderKind::Scratch,
                path: "/data/scratch".to_owned(),
                status: Status::Waiting,
            },
        ];
        let suggested = [
            Suggestion {
                path: "/home/me/Documents/dev/tmp".to_owned(),
                why: Why::Approved(7),
                proposable: true,
            },
            Suggestion {
                path: "/data/my tmp".to_owned(),
                why: Why::TempDir,
                proposable: true,
            },
        ];
        let text = lines(&rows, &suggested, HOME).join("\n");
        for expected in [
            "~/Documents/dev",
            "counts (your phone signed it)",
            "/tmp, /var/tmp, /dev/shm, ~/.cache",
            "built in",
            "waiting for you to confirm it on your phone",
            "~/Documents/dev/tmp  (you approved 7 deletes in it in the last 14 days)",
            "reins folders add --scratch ~/Documents/dev/tmp",
            "reins folders add --scratch '/data/my tmp'",
            "Confirm on your phone: Settings › Computers › this computer › Folders.",
        ] {
            assert!(text.contains(expected), "{expected}:\n{text}");
        }
        // Nothing waiting, nothing suggested: neither is mentioned.
        let quiet = lines(&rows[..1], &[], HOME).join("\n");
        assert!(!quiet.contains("Suggested") && !quiet.contains("Confirm on your phone"), "{quiet}");
    }
}
