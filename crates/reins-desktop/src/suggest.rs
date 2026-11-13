//! Scratch folders this computer suggests, learned from what the owner approved here.
//!
//! When the owner approves the same kind of delete on the phone again and again (`rm -rf ~/Documents/dev/tmp/<x>`), the
//! folder those deletes are in is probably one where deleting loses nothing. Reins then offers to stop asking: the
//! folder is suggested as a scratch folder, to the phone as a question card
//! ([`reins_proto::desktop::FolderProposal::suggested_scratch`]) and in `reins folders`.
//!
//! Nothing here makes anything count. A suggestion is in no configuration and no decision reads it. The folder counts
//! once the owner confirms it on the phone (the owner check; the phone then signs it as scratch for this computer), or
//! adds it here themselves (`reins folders add --scratch`).
//!
//! How a folder becomes a suggestion ([`candidates`], then [`settle`]):
//!
//! - Only commands the owner approved on the phone in the last [`WINDOW_SECS`] are read ([`asked`]): never what this
//!   computer's settings, Auto, Autopilot or a mode let through, never a request that timed out.
//! - Each command is read again by the consequence engine against today's workspace. What a command says in words
//!   counts for nothing: only a path the engine reads it deleting, in a command it read whole and with certainty, and
//!   only where the delete is at the top level for where the path is (a project folder's file git can't bring back, or
//!   the home folder).
//! - The folder holding such paths is suggested when at least [`MIN_APPROVALS`] approved commands deleted at least
//!   [`MIN_CHILDREN`] different things directly in it, and nothing in or over it was denied in that time.
//! - Never: a root or a folder holding one, a folder that is scratch already, one `refusal` refuses (the home folder,
//!   keys, Reins' own state), one in a git repository, one that isn't there.
//!
//! The daemon's own `$TMPDIR` is suggested the same way when it is none of the built-in temp folders.
//!
//! The activity log is a file an agent of the same user can write (a write outside the work folders, so Auto asks
//! about it). All it gains by writing approvals into it is a card on the phone, with the folder exactly as it is, for
//! the owner to confirm or not.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use reins_policy::desktop_folders::{inside, refusal};
use reins_policy::effects::{self, Area, Certainty, Context, Effect, PathRef, Workspace};

use crate::config::{Config, Paths};
use crate::journal::{Decider, Entry, Kind, Outcome};

/// How far back approvals are read: two weeks. Long enough for a habit to show (cleaning a temp folder a few times a
/// week), short enough that a folder used differently now is forgotten.
pub const WINDOW_SECS: i64 = 14 * 86_400;
/// The fewest commands the owner approved, each deleting in the folder, before it is suggested. One approval is an
/// answer to one request; three are a habit.
pub const MIN_APPROVALS: usize = 3;
/// The fewest different things, directly in the folder, those commands deleted. The same path approved three times
/// (a build folder made again each time) says something about that path, not about the folder around it.
pub const MIN_CHILDREN: usize = 3;
/// The most folders suggested at a time, the most approved first: each is a card the owner has to read.
pub const MAX_SUGGESTIONS: usize = 5;
/// The most commands read again, newest first.
const MAX_COMMANDS: usize = 1_000;
/// How far ahead of this computer's clock an entry's time may be.
const SKEW_SECS: i64 = 300;
/// How long reading the log again may take before nothing is suggested this time.
const DEADLINE: Duration = Duration::from_secs(10);

/// How a command the log kept was answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// The owner approved it on the phone.
    Owner,
    /// It was denied.
    Denied,
}

/// One command of the activity log, as the suggestions read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asked {
    pub command: String,
    /// Where it ran.
    pub cwd: String,
    pub answer: Answer,
}

/// The command and its working directory in an entry's detail, as the hooks write it: headers, `Command:` and the
/// command, maybe `Netted command:` and its copy, then `In: <cwd>`. `None` when the detail was cut, or when a part is
/// there twice (a command that writes such a line itself can't be told from the hook's).
fn command_and_cwd(detail: &str) -> Option<(String, String)> {
    const COMMAND: &str = "Command:\n";
    const NETTED: &str = "\n\nNetted command:\n";
    const IN: &str = "\n\nIn: ";
    if detail.ends_with('\u{2026}') {
        return None;
    }
    let start = if detail.starts_with(COMMAND) {
        COMMAND.len()
    } else {
        detail.find("\n\nCommand:\n")? + 2 + COMMAND.len()
    };
    let rest = &detail[start..];
    if rest.matches(IN).count() != 1 || rest.matches(NETTED).count() > 1 || rest.contains("\n\nCommand:\n") {
        return None;
    }
    let (body, cwd) = rest.rsplit_once(IN)?;
    if !cwd.starts_with('/') || cwd.contains('\n') {
        return None;
    }
    let command = body.split_once(NETTED).map_or(body, |(command, _)| command);
    Some((command.to_owned(), cwd.to_owned()))
}

/// The commands of the activity log (`journal`: its lines) from the last [`WINDOW_SECS`] that the owner approved on
/// the phone, or that were denied; newest first, at most [`MAX_COMMANDS`].
///
/// "The owner approved" is read strictly: the phone answered, it said yes, and the entry names nobody else as having
/// decided (Autopilot, Overnight, Bypass). An attribution in a shape this version doesn't know is someone else, never
/// the owner.
#[must_use]
pub fn asked(journal: &str, now: i64) -> Vec<Asked> {
    // A request is written when it is asked and again when it ends: its last line says how it ended.
    let mut order: Vec<String> = Vec::new();
    let mut last: HashMap<String, serde_json::Value> = HashMap::new();
    for line in journal.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(id) = value.get("id").and_then(serde_json::Value::as_str).map(str::to_owned) else {
            continue;
        };
        if !last.contains_key(&id) {
            order.push(id.clone());
        }
        last.insert(id, value);
    }
    let mut out = Vec::new();
    for id in order.iter().rev() {
        let Some(value) = last.remove(id) else {
            continue;
        };
        let attributed = value.get("autopilot").is_some_and(|a| !a.is_null());
        let Ok(entry) = serde_json::from_value::<Entry>(value) else {
            continue;
        };
        if entry.kind != Kind::Command || entry.at < now - WINDOW_SECS || entry.at > now + SKEW_SECS {
            continue;
        }
        let answer = match (entry.outcome, entry.decider) {
            (Outcome::Denied, _) => Answer::Denied,
            (Outcome::Approved, Decider::Phone) if !attributed && entry.autopilot.is_none() => Answer::Owner,
            _ => continue,
        };
        let Some((command, cwd)) = entry.detail.as_deref().and_then(command_and_cwd) else {
            continue;
        };
        out.push(Asked {
            command,
            cwd,
            answer,
        });
        if out.len() >= MAX_COMMANDS {
            break;
        }
    }
    out
}

/// What the suggestions know of this computer besides its activity log.
#[derive(Clone, Copy, Debug)]
pub struct Known<'a> {
    /// The home folder, as the engine reads `~` (empty when not known: then nothing is suggested).
    pub home: &'a str,
    /// `[workspace] roots` and `scratch`, as configured here.
    pub roots: &'a [String],
    pub scratch: &'a [String],
    /// What the phone signed for this computer (empty without a snapshot).
    pub signed_roots: &'a [String],
    pub signed_scratch: &'a [String],
}

impl Known<'_> {
    /// Every root and scratch folder, configured here or signed.
    fn all(&self) -> (Vec<String>, Vec<String>) {
        let both = |a: &[String], b: &[String]| {
            let mut all = a.to_vec();
            all.extend(b.iter().filter(|e| !a.contains(e)).cloned());
            all
        };
        (both(self.roots, self.signed_roots), both(self.scratch, self.signed_scratch))
    }

    /// The workspace commands are read against: every root and scratch folder, and the worktrees `cwd` is in.
    fn workspace(&self, worktrees: Vec<String>) -> Workspace {
        let (roots, scratch) = self.all();
        Workspace {
            home: self.home.trim_end_matches('/').to_owned(),
            roots,
            scratch,
            worktrees,
            tracked: Vec::new(),
        }
    }

    /// `path` (absolute, as the engine normalized it) may be suggested at all: it isn't refused, isn't scratch
    /// already (built in, configured or signed), and isn't a root nor a folder holding one (a root's projects must
    /// never become scratch).
    #[must_use]
    pub fn may_suggest(&self, path: &str) -> bool {
        let (roots, _) = self.all();
        !self.home.is_empty()
            && path.starts_with('/')
            && !path.contains(['*', '?', '['])
            && refusal(path, self.home).is_none()
            && self.workspace(Vec::new()).area(path) != Area::Scratch
            && !roots.iter().any(|r| inside(path, r, self.home))
    }
}

/// A folder the owner's approvals point at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub path: String,
    /// How many approved commands deleted in it.
    pub approvals: usize,
}

/// A delete is at the top level only for where the path is: a project folder's file git can't bring back, or the home
/// folder outside the workspace. (Elsewhere the place isn't the reason, or isn't one a scratch folder would change.)
fn asked_for_its_place(p: &PathRef) -> bool {
    !p.recoverable && matches!(p.area, Area::Source | Area::Home)
}

/// The folder a deleted path is directly in, when the path is one the engine knows exactly.
fn parent_of(path: &str) -> Option<&str> {
    if !path.starts_with('/') || path.contains(['*', '?', '[', '\u{fffd}']) {
        return None;
    }
    let (parent, child) = path.rsplit_once('/')?;
    (!parent.is_empty() && !child.is_empty()).then_some(parent)
}

/// `path` is `folder` or inside it (both absolute and normalized).
fn within(path: &str, folder: &str) -> bool {
    path == folder || path.strip_prefix(folder).is_some_and(|rest| rest.starts_with('/'))
}

/// The folders `asked` points at, the most approved first, at most [`MAX_SUGGESTIONS`]: see the module docs.
/// `worktrees` says which git worktrees a working directory is in (found on disk by the caller): what git can bring
/// back was never asked for its place.
///
/// Pure: nothing is read from disk here. [`settle`] checks the folders against the disk.
#[must_use]
pub fn candidates(asked: &[Asked], known: &Known<'_>, worktrees: &dyn Fn(&str) -> Vec<String>) -> Vec<Candidate> {
    // Per folder: the approved commands that deleted in it, and what they deleted there.
    let mut seen: BTreeMap<String, (BTreeSet<usize>, BTreeSet<String>)> = BTreeMap::new();
    let mut denied: Vec<String> = Vec::new();
    for (i, a) in asked.iter().enumerate() {
        // Only `HOME` is known of the environment the command ran in: any other variable in a path leaves it unknown.
        let ctx = Context::new(&a.cwd, known.workspace(worktrees(&a.cwd)));
        let analysis = effects::analyze_command(&a.command, &ctx);
        let deleted = analysis.effects.iter().flat_map(|e| match e {
            Effect::FileDelete {
                paths,
                ..
            } => paths.as_slice(),
            _ => &[],
        });
        match a.answer {
            Answer::Denied => denied.extend(deleted.map(|p| p.path.clone())),
            Answer::Owner if analysis.certainty == Certainty::Full && analysis.floor.is_empty() => {
                for p in deleted.filter(|p| asked_for_its_place(p)) {
                    if let Some(parent) = parent_of(&p.path) {
                        let (commands, children) = seen.entry(parent.to_owned()).or_default();
                        commands.insert(i);
                        children.insert(p.path.clone());
                    }
                }
            }
            Answer::Owner => {}
        }
    }
    let mut out: Vec<Candidate> = seen
        .into_iter()
        .filter(|(folder, (commands, children))| {
            commands.len() >= MIN_APPROVALS
                && children.len() >= MIN_CHILDREN
                && known.may_suggest(folder)
                // A "no" to a delete in the folder, of it, or of a folder holding it: the owner minds.
                && !denied.iter().any(|d| within(d, folder) || within(folder, d))
        })
        .map(|(path, (commands, _))| Candidate {
            path,
            approvals: commands.len(),
        })
        .collect();
    out.sort_by(|a, b| b.approvals.cmp(&a.approvals).then_with(|| a.path.cmp(&b.path)));
    out.truncate(MAX_SUGGESTIONS);
    out
}

/// Why a folder is suggested.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// The owner approved this many commands deleting in it, in the last [`WINDOW_SECS`].
    Approved(usize),
    /// It is the temp folder (`$TMPDIR`) of the program that looked.
    TempDir,
}

/// A scratch folder Reins suggests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    /// The folder, as commands name it (absolute, `~` expanded).
    pub path: String,
    pub why: Why,
    /// The path is the folder's own resolved form (no link in it), so the phone can sign it as it is and it then
    /// counts here. One reached through a link can only be added here (`reins folders add --scratch`), where both of
    /// its forms are known.
    pub proposable: bool,
}

/// What the disk says of a folder: `None` when it isn't a folder there or is in a git repository; else what it
/// resolves to.
pub type Disk<'a> = &'a dyn Fn(&str) -> Option<String>;

/// The suggestions among `candidates` and the temp folder `tmpdir` (`$TMPDIR`, when set): those that are folders on
/// this computer, outside any git repository, and not refused as they resolve (`resolved_home`: the home folder,
/// resolved). The temp folder first, at most [`MAX_SUGGESTIONS`] in all.
#[must_use]
pub fn settle(
    candidates: &[Candidate],
    tmpdir: Option<&str>,
    known: &Known<'_>,
    resolved_home: &str,
    disk: Disk<'_>,
) -> Vec<Suggestion> {
    let temp = tmpdir
        .map(str::trim)
        .filter(|t| t.starts_with('/') && !t.split('/').any(|part| part == ".."))
        .map(|t| effects::normalize(t, "/", ""))
        .filter(|t| known.may_suggest(t));
    let all = temp
        .into_iter()
        .map(|path| (path, Why::TempDir))
        .chain(candidates.iter().map(|c| (c.path.clone(), Why::Approved(c.approvals))));
    let mut out: Vec<Suggestion> = Vec::new();
    for (path, why) in all {
        let Some(resolved) = disk(&path) else {
            continue;
        };
        if refusal(&resolved, resolved_home).is_some() || out.iter().any(|s| s.path == path) {
            continue;
        }
        out.push(Suggestion {
            proposable: resolved == path,
            path,
            why,
        });
        if out.len() >= MAX_SUGGESTIONS {
            break;
        }
    }
    out
}

/// `folder` or a folder above it, below `stop` (the home folder; `/` for a folder outside it), holds `.git`: it is a
/// repository, or part of one's files.
fn in_repository(folder: &Path, stop: &Path) -> bool {
    folder.ancestors().take_while(|d| *d != stop && d.parent().is_some()).any(|d| d.join(".git").exists())
}

/// [`Disk`] on this computer.
fn on_disk(path: &str) -> Option<String> {
    let home_dir = crate::config::home_dir().ok().and_then(|h| h.to_str().map(str::to_owned)).unwrap_or_default();
    if !Path::new(path).is_dir() {
        return None;
    }
    let resolved = crate::folders::canonical(path, &home_dir)?;
    let resolved_home = crate::folders::home();
    let repository = [(path, home_dir.as_str()), (resolved.as_str(), resolved_home.as_str())]
        .iter()
        .any(|(folder, home)| in_repository(Path::new(folder), Path::new(home)));
    (!repository).then_some(resolved)
}

/// What this computer suggests now, read from its activity log, its configuration, the policy the phone signed for it
/// (when it holds one) and `tmpdir` (`$TMPDIR` of the program asking).
fn read(paths: &Paths, config: &Config, tmpdir: Option<&str>, now: i64) -> Vec<Suggestion> {
    let home = crate::config::home_dir().ok().and_then(|h| h.to_str().map(str::to_owned)).unwrap_or_default();
    let (_, held) = crate::policy::effective(paths, config, now);
    let (signed_roots, signed_scratch) = held.policy.map(|p| (p.roots, p.scratch)).unwrap_or_default();
    let known = Known {
        home: &home,
        roots: &config.workspace.roots,
        scratch: &config.workspace.scratch,
        signed_roots: &signed_roots,
        signed_scratch: &signed_scratch,
    };
    let asked = asked(&crate::journal::text(paths), now);
    let worktrees = |cwd: &str| crate::consequences::workspace(config, Some(Path::new(cwd))).worktrees;
    let found = candidates(&asked, &known, &worktrees);
    settle(&found, tmpdir, &known, &crate::folders::home(), &on_disk)
}

/// The readings running now.
static READING: AtomicUsize = AtomicUsize::new(0);
/// The most readings at a time: readings that outlive their deadline (the daemon looks every minute) never pile up.
const MAX_READINGS: usize = 4;

/// What this computer suggests now (see the module docs). The commands are read on a thread of the engine's own
/// (its stack, a panic caught), for at most [`DEADLINE`]; when that fails, nothing is suggested this time.
#[must_use]
pub fn suggestions(paths: &Paths, config: &Config, now: i64) -> Vec<Suggestion> {
    if READING.fetch_add(1, Ordering::AcqRel) >= MAX_READINGS {
        READING.fetch_sub(1, Ordering::AcqRel);
        return Vec::new();
    }
    let (paths, config) = (paths.clone(), config.clone());
    let tmpdir = std::env::var("TMPDIR").ok();
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("reins-suggest".to_owned())
        .stack_size(crate::consequences::STACK)
        .spawn(move || {
            let found = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                read(&paths, &config, tmpdir.as_deref(), now)
            }));
            READING.fetch_sub(1, Ordering::AcqRel);
            let _sent = tx.send(found.unwrap_or_default());
        });
    if spawned.is_err() {
        READING.fetch_sub(1, Ordering::AcqRel);
        return Vec::new();
    }
    rx.recv_timeout(DEADLINE).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/home/me";
    const TMP: &str = "/home/me/Documents/dev/tmp";
    const NOW: i64 = 1_800_000_000;

    fn roots() -> Vec<String> {
        crate::config::WorkspaceConfig::default().roots
    }

    fn known<'a>(roots: &'a [String], scratch: &'a [String]) -> Known<'a> {
        Known {
            home: HOME,
            roots,
            scratch,
            signed_roots: &[],
            signed_scratch: &[],
        }
    }

    fn approved(command: &str) -> Asked {
        Asked {
            command: command.to_owned(),
            cwd: "/home/me/Documents/dev".to_owned(),
            answer: Answer::Owner,
        }
    }

    fn denied(command: &str) -> Asked {
        Asked {
            answer: Answer::Denied,
            ..approved(command)
        }
    }

    fn no_worktrees(_: &str) -> Vec<String> {
        Vec::new()
    }

    /// The folders suggested for `asked` with the standard roots and no scratch folder.
    fn suggested(asked: &[Asked]) -> Vec<(String, usize)> {
        let roots = roots();
        candidates(asked, &known(&roots, &[]), &no_worktrees).into_iter().map(|c| (c.path, c.approvals)).collect()
    }

    fn three(folder: &str) -> Vec<Asked> {
        ["a", "b", "c"].iter().map(|x| approved(&format!("rm -rf {folder}/{x}"))).collect()
    }

    #[test]
    fn three_approvals_of_three_things_in_one_folder_suggest_it() {
        assert_eq!(suggested(&three("~/Documents/dev/tmp")), [(TMP.to_owned(), 3)]);
        // Two approvals are not a habit.
        assert!(suggested(&three("~/Documents/dev/tmp")[..2]).is_empty());
        // The same thing three times says nothing about the folder around it.
        let same: Vec<Asked> = (0..5).map(|_| approved("rm -rf ~/Documents/dev/tmp/build")).collect();
        assert!(suggested(&same).is_empty());
        // One command deleting three things is one approval.
        assert!(
            suggested(&[approved("rm -rf ~/Documents/dev/tmp/a ~/Documents/dev/tmp/b ~/Documents/dev/tmp/c")])
                .is_empty()
        );
        // Relative paths are read from where the command ran; a command of several parts is read whole.
        let mut relative = vec![
            approved("rm -rf tmp/a; ls tmp"),
            approved("cd tmp && rm -rf b"),
            approved("rm -r ./tmp/c && echo done"),
        ];
        assert_eq!(suggested(&relative), [(TMP.to_owned(), 3)]);
        // Only direct children count: deeper deletes are about their own folder.
        relative[2] = approved("rm -rf tmp/c/deep/er");
        assert!(suggested(&relative).is_empty());
        // In the home folder outside the workspace, too.
        assert_eq!(suggested(&three("~/Downloads/unpacked")), [("/home/me/Downloads/unpacked".to_owned(), 3)]);
    }

    #[test]
    fn a_denied_delete_in_or_over_the_folder_blocks_it() {
        let mut asked = three("~/Documents/dev/tmp");
        asked.push(denied("rm -rf ~/Documents/dev/tmp/keep-this"));
        assert!(suggested(&asked).is_empty());
        // Deeper in it, the folder itself, a folder holding it: all a "no" about this place.
        for no in [
            "rm -rf ~/Documents/dev/tmp/a/b/c",
            "rm -rf ~/Documents/dev/tmp",
            "rm -rf ~/Documents/dev",
            "cd ~/Documents/dev/tmp && rm -rf x",
        ] {
            let mut asked = three("~/Documents/dev/tmp");
            asked.push(denied(no));
            assert!(suggested(&asked).is_empty(), "{no}");
        }
        // A "no" somewhere else doesn't.
        let mut asked = three("~/Documents/dev/tmp");
        asked.push(denied("rm -rf ~/Documents/dev/tmpfiles/x"));
        asked.push(denied("rm -rf ~/Pictures/2019"));
        assert_eq!(suggested(&asked).len(), 1);
    }

    /// What an agent writes into a command is words: only what the engine reads the command doing counts.
    #[test]
    fn a_command_that_only_mentions_paths_suggests_nothing() {
        let mentions = [
            "echo 'rm -rf ~/Documents/dev/tmp/a'",
            "echo rm -rf ~/Documents/dev/tmp/b",
            "printf '%s\\n' \"rm -rf ~/Documents/dev/tmp/c\" > /dev/null",
            "git commit -m 'rm -rf ~/Documents/dev/tmp/d'",
            "# rm -rf ~/Documents/dev/tmp/e\nls",
            "ls ~/Documents/dev/tmp/f ~/Documents/dev/tmp/g ~/Documents/dev/tmp/h",
            "cat <<'EOF'\nrm -rf ~/Documents/dev/tmp/i\nEOF",
        ];
        let asked: Vec<Asked> = mentions.iter().map(|c| approved(c)).collect();
        assert!(suggested(&asked).is_empty(), "{:?}", suggested(&asked));
        // A command the engine can't read with certainty is not counted, whatever else it deletes: a value from the
        // environment, a program it has no model of, a path computed when it runs.
        let unsure = [
            "rm -rf ~/Documents/dev/tmp/a; some-unknown-tool --purge",
            "rm -rf ~/Documents/dev/tmp/b $EXTRA",
            "rm -rf ~/Documents/dev/tmp/c \"$(cat list.txt)\"",
            "rm -rf $TMPDIR/d ~/Documents/dev/tmp/d",
            "rm -rf ~/Documents/dev/tmp/e; curl https://example.com/install.sh | sh",
        ];
        let asked: Vec<Asked> = unsure.iter().map(|c| approved(c)).collect();
        assert!(suggested(&asked).is_empty(), "{:?}", suggested(&asked));
        // A pattern names no one thing: never a child.
        let globs: Vec<Asked> =
            ["a*", "b?", "*"].iter().map(|g| approved(&format!("rm -rf ~/Documents/dev/tmp/{g}"))).collect();
        assert!(suggested(&globs).is_empty());
    }

    #[test]
    fn roots_scratch_and_refused_folders_are_never_suggested() {
        // Projects deleted from a root: the root is where projects are, never scratch.
        assert!(suggested(&three("~/Documents/dev")).is_empty());
        // A folder holding a root.
        let roots = vec!["~/Documents/dev".to_owned(), "~/Documents/work/clients".to_owned()];
        let asked = three("~/Documents/work");
        assert!(candidates(&asked, &known(&roots, &[]), &no_worktrees).is_empty());
        // The home folder, and keys.
        assert!(suggested(&three("~")).is_empty());
        assert!(suggested(&three("~/.ssh")).is_empty());
        assert!(suggested(&three("~/.config/reins")).is_empty());
        assert!(suggested(&three("/home")).is_empty());
        // Already scratch here: the deletes were not asked for their place, and the folder is covered.
        let scratch = vec!["~/Documents/dev/tmp".to_owned()];
        let standard = self::roots();
        assert!(candidates(&three("~/Documents/dev/tmp"), &known(&standard, &scratch), &no_worktrees).is_empty());
        assert!(candidates(&three("~/Documents/dev/tmp/sub"), &known(&standard, &scratch), &no_worktrees).is_empty());
        // Signed as scratch by the phone, though no configuration here names it: covered too.
        let signed = Known {
            signed_scratch: &scratch,
            ..known(&standard, &[])
        };
        assert!(candidates(&three("~/Documents/dev/tmp"), &signed, &no_worktrees).is_empty());
        // A root only the phone signed is a root.
        let signed_root = vec!["/mnt/data/dev".to_owned()];
        let k = Known {
            signed_roots: &signed_root,
            ..known(&standard, &[])
        };
        assert!(candidates(&three("/mnt/data/dev"), &k, &no_worktrees).is_empty());
        assert_eq!(candidates(&three("/mnt/data/dev/tmp"), &k, &no_worktrees).len(), 1);
        // The built-in temp folders are scratch already; outside the home folder and the workspace nothing is.
        assert!(suggested(&three("/tmp/work")).is_empty());
        assert!(suggested(&three("~/.cache/thing")).is_empty());
        assert!(suggested(&three("/opt/data/tmp")).is_empty());
        // Without the home folder nothing is suggested.
        let homeless = Known {
            home: "",
            ..known(&standard, &[])
        };
        assert!(candidates(&three("/home/me/Documents/dev/tmp"), &homeless, &no_worktrees).is_empty());
    }

    #[test]
    fn what_git_can_bring_back_was_not_asked_for_its_place() {
        // Deletes inside the worktree the command ran in are recoverable: they say nothing about a scratch folder.
        let in_app = |x: &str| Asked {
            cwd: "/home/me/Documents/dev/app".to_owned(),
            ..approved(&format!("rm -rf scratch/{x}"))
        };
        let asked = vec![in_app("a"), in_app("b"), in_app("c")];
        let roots = roots();
        let app = |cwd: &str| {
            if cwd.starts_with("/home/me/Documents/dev/app") {
                vec!["/home/me/Documents/dev/app".to_owned()]
            } else {
                Vec::new()
            }
        };
        assert!(candidates(&asked, &known(&roots, &[]), &app).is_empty());
        // Build output is never asked for its place either.
        assert!(suggested(&three("~/Documents/dev/app/target")).is_empty());
    }

    #[test]
    fn the_most_approved_first_and_at_most_five() {
        let mut asked = Vec::new();
        for (folder, times) in [("p", 3), ("q", 5), ("r", 4), ("s", 3), ("t", 3), ("u", 6), ("v", 3)] {
            for i in 0..times {
                asked.push(approved(&format!("rm -rf ~/Downloads/{folder}/{i}")));
            }
        }
        let got = suggested(&asked);
        assert_eq!(got.len(), MAX_SUGGESTIONS);
        assert_eq!(got[0], ("/home/me/Downloads/u".to_owned(), 6));
        assert_eq!(got[1], ("/home/me/Downloads/q".to_owned(), 5));
        assert_eq!(got[2], ("/home/me/Downloads/r".to_owned(), 4));
        assert_eq!(got[3].0, "/home/me/Downloads/p", "then by name");
    }

    fn line(id: &str, at: i64, outcome: &str, decider: &str, command: &str, more: &str) -> String {
        let detail = format!(
            "Rule: Recursive deletes · rm -r\nMatched: {command}\nGroup: deletes (destructive)\n\nCommand:\n\
             {command}\n\nIn: /home/me/Documents/dev"
        );
        let entry = serde_json::json!({"id": id, "at": at, "kind": "command", "source": "Claude Code",
            "what": format!("Claude Code wants to run: {command}"), "detail": detail, "decider": decider,
            "outcome": outcome});
        let text = entry.to_string();
        if more.is_empty() {
            text
        } else {
            // The entry with the fields of `more` written into it as they are.
            format!("{},{more}}}", &text[..text.len() - 1])
        }
    }

    #[test]
    fn only_what_the_owner_approved_on_the_phone_is_read() {
        let rm = |x: &str| format!("rm -rf ~/Documents/dev/tmp/{x}");
        let log = [
            // Asked, then approved: the last line of a request counts.
            line("1", NOW - 100, "waiting", "phone", &rm("a"), ""),
            line("1", NOW - 100, "approved", "phone", &rm("a"), ""),
            // Let through by this computer's settings or Auto, approved here in local mode, timed out, failed.
            line("2", NOW - 90, "allowed", "settings", &rm("b"), ""),
            line("3", NOW - 80, "approved", "local", &rm("c"), ""),
            line("4", NOW - 70, "timed_out", "phone", &rm("d"), ""),
            line("5", NOW - 60, "failed", "phone", &rm("e"), ""),
            line("6", NOW - 50, "waiting", "phone", &rm("f"), ""),
            // Decided on the phone, but not by the owner; and an attribution this version can't read.
            line("7", NOW - 40, "approved", "phone", &rm("g"), r#""autopilot":{"by":"bypass","correctable":true}"#),
            line("8", NOW - 30, "approved", "phone", &rm("h"), r#""autopilot":{"by":"autopilot","correctable":true}"#),
            line("9", NOW - 20, "approved", "phone", &rm("i"), r#""autopilot":{"by":"something_new"}"#),
            line("10", NOW - 20, "approved", "phone", &rm("j"), r#""autopilot":"yes""#),
            // Too old, and from the future.
            line("11", NOW - WINDOW_SECS - 1, "approved", "phone", &rm("k"), ""),
            line("12", NOW + 86_400, "approved", "phone", &rm("l"), ""),
            // Denied: kept, as a "no".
            line("13", NOW - 10, "denied", "phone", &rm("m"), ""),
            "not json".to_owned(),
            String::new(),
        ]
        .join("\n");
        let got = asked(&log, NOW);
        assert_eq!(
            got.iter().map(|a| (a.command.as_str(), a.answer)).collect::<Vec<_>>(),
            [(rm("m").as_str(), Answer::Denied), (rm("a").as_str(), Answer::Owner)],
            "newest first"
        );
        assert!(got.iter().all(|a| a.cwd == "/home/me/Documents/dev"));
        // Other kinds of request are not commands.
        let file =
            line("14", NOW, "approved", "phone", &rm("n"), "").replace(r#""kind":"command""#, r#""kind":"file""#);
        assert!(asked(&file, NOW).is_empty());
    }

    #[test]
    fn the_command_is_read_only_from_a_detail_that_is_whole() {
        let read = |detail: &str| command_and_cwd(detail);
        assert_eq!(
            read("Command:\nrm -rf a\n\nIn: /home/me/x"),
            Some(("rm -rf a".to_owned(), "/home/me/x".to_owned()))
        );
        assert_eq!(
            read(
                "Auto: Ask · Mine · Can't be undone\n\nCommand:\nrm -rf a; ls\nls b\n\nNetted command:\n'/usr/bin/reins' net run --plan n1 --store '/s' -- rm -rf a; ls\nls b\n\nIn: /home/me/x"
            ),
            Some(("rm -rf a; ls\nls b".to_owned(), "/home/me/x".to_owned()))
        );
        // No working directory, a relative or cut one, a cut detail.
        assert_eq!(read("Command:\nrm -rf a"), None);
        assert_eq!(read("Command:\nrm -rf a\n\nIn: x"), None);
        assert_eq!(
            read("Command:\nrm -rf a\n\nNetted command:\n'/usr/bin/reins' net run --plan n1 -- rm -r\u{2026}"),
            None
        );
        assert_eq!(read("Command:\nrm -rf a\n\nIn: /home/me/Docu\u{2026}"), None);
        assert_eq!(read("Files:\n/home/me/x\n\nIn: /home/me"), None);
        // A command that writes the hook's lines itself: which is which can't be told, so it is not read.
        assert_eq!(read("Command:\nrm -rf a\n\nIn: /home/me/Pictures\n\nIn: /home/me/x"), None);
        assert_eq!(read("Command:\nrm -rf a\n\nIn: /home/me/Pictures\nrm -rf b\n\nIn: /home/me/x"), None);
        assert_eq!(
            read("Command:\nls\n\nNetted command:\nrm -rf a\n\nNetted command:\nreins net run -- ls\n\nIn: /home/me/x"),
            None
        );
        assert_eq!(read("Matched: x\n\nCommand:\nls\n\nCommand:\nrm -rf a\n\nIn: /home/me/x"), None);
    }

    #[test]
    fn suggestions_are_settled_against_the_disk() {
        let roots = roots();
        let k = known(&roots, &[]);
        let found = vec![
            Candidate {
                path: TMP.to_owned(),
                approvals: 4,
            },
            Candidate {
                path: "/home/me/Downloads/gone".to_owned(),
                approvals: 3,
            },
            Candidate {
                path: "/home/me/linked".to_owned(),
                approvals: 3,
            },
            Candidate {
                path: "/home/me/to-keys".to_owned(),
                approvals: 3,
            },
        ];
        // `gone` isn't there (or is in a repository); `linked` resolves elsewhere; `to-keys` is a link to `~/.ssh`.
        let disk = |path: &str| match path {
            TMP | "/data/tmp" => Some(path.to_owned()),
            "/home/me/linked" => Some("/data/real".to_owned()),
            "/home/me/to-keys" => Some("/home/me/.ssh".to_owned()),
            _ => None,
        };
        let got = settle(&found, Some("/data/tmp/"), &k, HOME, &disk);
        assert_eq!(
            got,
            [
                Suggestion {
                    path: "/data/tmp".to_owned(),
                    why: Why::TempDir,
                    proposable: true
                },
                Suggestion {
                    path: TMP.to_owned(),
                    why: Why::Approved(4),
                    proposable: true
                },
                Suggestion {
                    path: "/home/me/linked".to_owned(),
                    why: Why::Approved(3),
                    proposable: false
                },
            ]
        );
        // The temp folder is not suggested when it is built in, relative, covered, a root, refused, or has `..`.
        let scratch = vec!["/data/tmp".to_owned()];
        let all = |_: &str| Some("/data/tmp".to_owned());
        let covered = known(&roots, &scratch);
        for (tmpdir, k) in [
            ("/tmp", k),
            ("/tmp/claude", k),
            ("/var/tmp/x", k),
            ("/home/me/.cache/tmp", k),
            ("tmp", k),
            ("", k),
            ("/data/tmp", covered),
            ("/data/tmp/sub", covered),
            ("/home/me/Documents/dev", k),
            ("/home/me/Documents", k),
            ("/home/me", k),
            ("/", k),
            ("/home/me/.ssh/tmp", k),
            ("/data/tmp/../../home/me/x", k),
        ] {
            assert!(settle(&[], Some(tmpdir), &k, HOME, &all).is_empty(), "{tmpdir}");
        }
        // The temp folder and a learned folder that are the same folder: once.
        let same = [Candidate {
            path: "/data/tmp".to_owned(),
            approvals: 3,
        }];
        assert_eq!(settle(&same, Some("/data/tmp"), &k, HOME, &disk).len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_in_a_repository_is_found_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let home = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir_all(home.join("dev/app/.git")).unwrap();
        std::fs::create_dir_all(home.join("dev/app/scratch/deep")).unwrap();
        std::fs::create_dir_all(home.join("dev/tmp/x")).unwrap();
        assert!(in_repository(&home.join("dev/app"), &home));
        assert!(in_repository(&home.join("dev/app/scratch/deep"), &home));
        assert!(!in_repository(&home.join("dev/tmp"), &home));
        assert!(!in_repository(&home.join("dev/tmp/x"), &home));
        // The home folder's own `.git` (dotfiles kept in git) is not looked at; a folder outside it is, up to `/`.
        std::fs::create_dir_all(home.join(".git")).unwrap();
        assert!(!in_repository(&home.join("dev/tmp"), &home));
        assert!(in_repository(&home.join("dev/tmp"), Path::new("/nowhere")));
    }

    /// `reins folders add` run by an agent is a command like any other: the engine has no model of it, so it is
    /// asked about.
    #[test]
    fn an_agent_adding_a_folder_itself_is_asked_about() {
        let ctx = Context::new("/home/me/Documents/dev/app", Workspace::standard(HOME));
        for command in [
            "reins folders add --scratch ~/Documents/dev/app",
            "reins folders temp ~/Pictures",
            "reins folders add ~/Pictures",
            "reins folders remove ~/Documents/dev",
        ] {
            let a = effects::analyze_command(command, &ctx);
            assert!(a.needs_human(), "{command}: {a:?}");
        }
    }
}
