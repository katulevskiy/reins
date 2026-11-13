//! A computer's reading of its own hook question, for the phone to decide it with
//! (`docs/design/decisions-v2/08-review-resolutions.md` R1).
//!
//! A computer decides hook requests with the policy the phone signed for it only while that copy is fresh
//! ([`crate::desktop_policy::VALIDITY_SECS`]). Past that it asks the phone, and sends what it read of the request
//! ([`Reading`]), so the phone's own matrix decides it as the computer would have with a fresh copy, and the owner is
//! asked only when the matrix asks.
//!
//! The phone takes from a reading what a fresh copy already trusts the computer for: what the command does (its
//! effects, read where the working directory, the environment and the git state are). The rest is the phone's own:
//!
//! - **the folders.** A reading names the workspace it was read with. The phone uses the reading only when every
//!   folder of that workspace counts under what it signs for that computer now ([`counts_as_root`],
//!   [`counts_as_scratch`]: what `desktop_policy::clamp_roots` and `clamp_scratch` let a fresh copy use), and it places
//!   each path again against those folders ([`accept`]);
//! - **the level and the floor.** Both are computed again from the effects ([`Analysis::new`]). The numbers the
//!   computer wrote are dropped, and so are its labels;
//! - **the question.** [`about`] binds a reading to the command (or the files), the folder and the netted command the
//!   phone is shown: a reading that is about anything else is not used;
//! - the matrix, the rules, the tasks, the facts and the mode. The computer's own `[auto]` settings travel with the
//!   reading and only tighten ([`crate::desktop_policy::tighten_with_local`]).
//!
//! Anything that doesn't hold (another version, too large, not JSON, a folder the phone didn't sign) and the reading is
//! not used at all: the question is asked as it was before readings.
//!
//! A computer sends a reading only to a phone whose signed snapshot says it takes them
//! ([`crate::desktop_policy::reading_format`]): a phone refuses a call with an argument it doesn't know.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::context::Facts;
use crate::desktop_folders::{inside, refusal, same};
use crate::effects::nets::netted_level;
use crate::effects::{Analysis, Area, Certainty, Effect, Level, PathRef, Workspace, normalize};
use crate::matrix::Matrix;

/// The format of a reading. A reading of another format is not used.
pub const FORMAT: u32 = 1;
/// The largest reading sent or read, as JSON: 16 KiB, the `reading` argument's own limit. One effect reads as about
/// 150 bytes, so that holds some hundred effects with the repository's remotes and the computer's own settings: far
/// more than the commands a hook asks about have (a handful), and of the size of the question itself (its detail
/// holds 8,000 characters). A command that reads as more is one the owner should see anyway: no reading is sent, and
/// the question is asked as before.
pub const MAX_BYTES: usize = reins_proto::connector::ASK_READING_MAX;

/// What a computer read of the request its hook asks about.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reading {
    pub v: u32,
    /// [`about`] of the request this was read from.
    pub about: String,
    /// The engine's analysis. Its `level`, `floor` and `labels` are never used: see [`accept`].
    pub analysis: Analysis,
    /// With a safety net the computer applies before the command runs: the paths the net keeps (`NetPlan`'s
    /// `netted_paths`). The phone levels the effects with them itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub netted: Option<Vec<String>>,
    /// The remotes of the repository the request acts in (name → URL), as the computer's strict reader of git's
    /// configuration read them; `None` when not known (then a push is Outside).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remotes: Option<BTreeMap<String, String>>,
    /// The workspace the request was read with: the computer's home folder, its roots and its scratch folders
    /// (absolute).
    pub home: String,
    #[serde(default)]
    pub roots: Vec<String>,
    #[serde(default)]
    pub scratch: Vec<String>,
    /// The computer's own `[auto]` matrix when it isn't the default, and its own owner, repository and host facts:
    /// they only tighten.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matrix: Option<Matrix>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facts: Option<Facts>,
}

impl Reading {
    /// The reading as it is sent; `None` when it is larger than [`MAX_BYTES`].
    #[must_use]
    pub fn to_wire(&self) -> Option<String> {
        serde_json::to_string(self).ok().filter(|t| t.len() <= MAX_BYTES)
    }

    /// The reading in `text`; `None` when it is too large, not a reading, or of another format.
    #[must_use]
    pub fn from_wire(text: &str) -> Option<Self> {
        if text.len() > MAX_BYTES {
            return None;
        }
        serde_json::from_str::<Self>(text).ok().filter(|r| r.v == FORMAT)
    }
}

/// What a hook asks about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asked<'a> {
    Command(&'a str),
    Files {
        write: bool,
        paths: &'a [&'a str],
    },
}

/// What binds a reading to its question: SHA-256 (lowercase hex) over what is asked about, the folder it runs in and
/// the netted command, each with its length. The computer makes it from the hook's raw call; the phone makes it again
/// from the question it shows. Where the two differ (a detail cut or cleaned on its way, another command), the reading
/// isn't about that question.
#[must_use]
pub fn about(asked: &Asked<'_>, cwd: Option<&str>, netted: Option<&str>) -> String {
    let mut h = Sha256::new();
    h.update(b"reins/hook-reading/v1\n");
    let mut part = |tag: &str, bytes: &[u8]| {
        h.update(tag.as_bytes());
        h.update(u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_be_bytes());
        h.update(bytes);
    };
    match asked {
        Asked::Command(c) => part("command", c.as_bytes()),
        Asked::Files {
            write,
            paths,
        } => {
            part(
                if *write {
                    "write"
                } else {
                    "read"
                },
                &u64::try_from(paths.len()).unwrap_or(u64::MAX).to_be_bytes(),
            );
            for p in *paths {
                part("path", p.as_bytes());
            }
        }
    }
    part(
        if cwd.is_some() {
            "cwd"
        } else {
            "no cwd"
        },
        cwd.unwrap_or_default().as_bytes(),
    );
    part(
        if netted.is_some() {
            "netted"
        } else {
            "no net"
        },
        netted.unwrap_or_default().as_bytes(),
    );
    data_encoding::HEXLOWER.encode(&h.finalize())
}

/// The folders signed for a computer: its home folder (empty when not known: then `~` can't be read, and a folder
/// written with it never counts), the roots and the scratch folders.
#[derive(Clone, Copy, Debug)]
pub struct Folders<'a> {
    pub home: &'a str,
    pub roots: &'a [String],
    pub scratch: &'a [String],
}

/// Whether `folder` (absolute) counts as a workspace root with `signed`: inside a signed root, and never a folder
/// that is refused everywhere (`desktop_folders::refusal`).
#[must_use]
pub fn counts_as_root(folder: &str, signed: &Folders<'_>) -> bool {
    absolute(folder)
        && refusal(folder, signed.home).is_none()
        && signed.roots.iter().any(|r| inside(r, folder, signed.home))
}

/// Whether `folder` (absolute) counts as a scratch folder with `signed`: signed as one, or strictly inside a signed
/// root (a root itself, or anything above one, never is).
#[must_use]
pub fn counts_as_scratch(folder: &str, signed: &Folders<'_>) -> bool {
    absolute(folder)
        && refusal(folder, signed.home).is_none()
        && (signed.scratch.iter().any(|s| absolute(&normalize(s, "/", signed.home)) && same(s, folder, signed.home))
            || strictly_inside_a_root(folder, signed))
}

/// An absolute path as the engine writes one: no `..` left in it.
fn absolute(path: &str) -> bool {
    path.starts_with('/') && !path.split('/').any(|p| p == "..")
}

fn strictly_inside_a_root(path: &str, signed: &Folders<'_>) -> bool {
    signed.roots.iter().any(|r| inside(r, path, signed.home) && !same(r, path, signed.home))
}

/// How many folders below the deepest signed root holding it `path` is (0: a root itself); `None` outside every root.
fn depth_below_root(path: &str, signed: &Folders<'_>) -> Option<usize> {
    let parts = |p: &str| p.split('/').filter(|s| !s.is_empty()).count();
    signed
        .roots
        .iter()
        .filter(|r| inside(r, path, signed.home))
        .map(|r| parts(&normalize(r, "/", signed.home)))
        .max()
        .map(|root| parts(path).saturating_sub(root))
}

/// A reading as far as the phone takes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Accepted {
    /// The effects with each path placed by the phone, levelled by the phone.
    pub analysis: Analysis,
    /// The same with the computer's safety net applied, when the question carries one.
    pub netted: Option<Analysis>,
    pub remotes: Option<BTreeMap<String, String>>,
    /// The computer's own settings, which only tighten.
    pub matrix: Option<Matrix>,
    pub facts: Option<Facts>,
}

/// What the phone takes from `reading` for a computer it signs `signed` for. `None`: the reading isn't used.
///
/// - The workspace it was read with must be one a fresh copy of the policy would have given the computer: its home
///   the one the phone knows, each root and scratch folder counting under `signed`. The engine reads more than paths
///   with the workspace (whether a script is the project's own, whether a database file is a local one), so a
///   workspace the phone didn't sign makes the whole reading unusable, not only its paths.
/// - Each path is then placed again ([`place`]), and the analysis built again from the effects: its level and its
///   floor are the phone's.
#[must_use]
pub fn accept(reading: &Reading, signed: &Folders<'_>) -> Option<Accepted> {
    if reading.v != FORMAT {
        return None;
    }
    let home = signed.home.trim_end_matches('/');
    if !home.is_empty() && reading.home.trim_end_matches('/') != home {
        return None;
    }
    let signed = Folders {
        home,
        ..*signed
    };
    if !reading.roots.iter().all(|r| counts_as_root(r, &signed))
        || !reading.scratch.iter().all(|s| counts_as_scratch(s, &signed))
    {
        return None;
    }
    let ws = Workspace {
        home: home.to_owned(),
        roots: signed.roots.to_vec(),
        scratch: signed.scratch.to_vec(),
        ..Workspace::default()
    };
    let effects: Vec<Effect> = reading.analysis.effects.iter().map(|e| placed(e, &ws, &signed)).collect();
    let analysis = Analysis::new(effects, reading.analysis.certainty, reading.analysis.unresolved.clone());
    let netted = reading.netted.as_ref().map(|paths| with_net(&analysis, paths));
    Some(Accepted {
        analysis,
        netted,
        remotes: reading.remotes.clone(),
        matrix: reading.matrix.clone(),
        facts: reading.facts.clone(),
    })
}

/// `plain` levelled with the paths a net keeps (as `effects::nets` levels a plan's `after`): the same effects,
/// certainty and floor; a delete or an overwrite the net keeps, where a net can lower it, is recoverable.
fn with_net(plain: &Analysis, netted: &[String]) -> Analysis {
    let level = plain.effects.iter().map(|e| netted_level(e, netted)).max().unwrap_or(Level::L0);
    let level = if plain.certainty == Certainty::Full {
        level
    } else {
        level.max(Level::L1)
    };
    Analysis {
        level,
        ..plain.clone()
    }
}

/// `e` with each of its paths placed by the phone.
fn placed(e: &Effect, ws: &Workspace, signed: &Folders<'_>) -> Effect {
    match e {
        Effect::FileRead {
            path,
            kind,
        } => Effect::FileRead {
            path: place(path, ws, signed),
            kind: *kind,
        },
        Effect::FileWrite {
            path,
        } => Effect::FileWrite {
            path: place(path, ws, signed),
        },
        Effect::FileDelete {
            paths,
            recursive,
        } => Effect::FileDelete {
            paths: paths.iter().map(|p| place(p, ws, signed)).collect(),
            recursive: *recursive,
        },
        other => other.clone(),
    }
}

/// Where the phone places a path the computer read, from the folders it signed:
///
/// - outside the signed workspace (the home folder, the system): the phone's area, whatever the computer said;
/// - inside it: the computer's finer word among scratch, build output and source is kept (it knows its worktrees, its
///   generated folders and its own scratch folders), as far as a fresh copy would let it: scratch or build output only
///   in a signed scratch folder or strictly inside a signed root, "git brings it back" only inside a project (two
///   folders below a root or deeper: a worktree is a folder strictly inside a root, and its own folder is never
///   recoverable). A stricter word (the home folder, the system, not known) is kept;
/// - a path on another computer keeps that: it is never a local path, and its level is the worst;
/// - a path the phone can't place (not absolute, a `..` left in it) is not known when the computer said so, else the
///   home folder.
///
/// "Not known" is kept only where it isn't looser than the phone's own placing: a write somewhere not known is
/// recoverable and Mine, a write in the home folder is neither.
#[must_use]
pub fn place(p: &PathRef, ws: &Workspace, signed: &Folders<'_>) -> PathRef {
    let at = |area: Area, recoverable: bool| PathRef {
        path: p.path.clone(),
        area,
        recoverable,
    };
    if matches!(p.area, Area::Remote(_)) {
        return at(p.area.clone(), false);
    }
    let own = if absolute(&p.path) {
        ws.area(&p.path)
    } else {
        Area::Unknown
    };
    match own {
        // Without the computer's home folder nothing is known to be the system's rather than the home's.
        Area::System if ws.home.is_empty() && p.area != Area::System => at(Area::Home, false),
        Area::Home | Area::System => at(own, false),
        Area::Unknown | Area::Remote(_) => at(
            if p.area == Area::Unknown {
                Area::Unknown
            } else {
                Area::Home
            },
            false,
        ),
        Area::Scratch | Area::Generated | Area::Source => {
            let area = match &p.area {
                Area::Scratch | Area::Generated if own == Area::Scratch || strictly_inside_a_root(&p.path, signed) => {
                    p.area.clone()
                }
                Area::Scratch | Area::Generated | Area::Source => Area::Source,
                stricter => stricter.clone(),
            };
            let recoverable = p.recoverable && depth_below_root(&p.path, signed).is_some_and(|d| d >= 2);
            at(area, recoverable)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{Base, FactKind, Zone, request_zone};
    use crate::effects::{FileKind, Floor, SecretKind};
    use crate::matrix::{Input, Outcome, Snapshot, evaluate};

    const HOME: &str = "/home/me";

    fn standard() -> Vec<String> {
        Workspace::standard("").roots
    }

    fn path(path: &str, area: Area, recoverable: bool) -> PathRef {
        PathRef {
            path: path.to_owned(),
            area,
            recoverable,
        }
    }

    fn delete(p: PathRef) -> Effect {
        Effect::FileDelete {
            paths: vec![p],
            recursive: true,
        }
    }

    fn reading(effects: Vec<Effect>) -> Reading {
        Reading {
            v: FORMAT,
            about: about(&Asked::Command("x"), None, None),
            analysis: Analysis::new(effects, Certainty::Full, Vec::new()),
            netted: None,
            remotes: None,
            home: HOME.to_owned(),
            roots: vec!["/home/me/Documents/dev".to_owned()],
            scratch: Vec::new(),
            matrix: None,
            facts: None,
        }
    }

    /// What the default matrix says of `a`, Mine facts for nobody.
    fn outcome(a: &Analysis) -> (Outcome, Zone, Level) {
        let snap = Snapshot::default();
        let r = request_zone(a, &snap.facts, &Base::default());
        let e = evaluate(
            &Input {
                analysis: a,
                resolved: &r,
                connection: "desk",
                seen_resource: false,
                now: 0,
                overnight: false,
                netted: false,
            },
            &snap,
        );
        (e.outcome, e.zone, e.level)
    }

    #[test]
    fn a_reading_round_trips_and_another_format_is_ignored() {
        let r = reading(vec![delete(path("/home/me/Documents/dev/app/target", Area::Generated, false))]);
        let wire = r.to_wire().unwrap();
        assert_eq!(Reading::from_wire(&wire), Some(r.clone()));
        let newer = wire.replacen("\"v\":1", "\"v\":2", 1);
        assert_ne!(newer, wire);
        assert_eq!(Reading::from_wire(&newer), None, "a format this build doesn't know");
        for bad in ["", "{}", "not json", "[1]", "{\"v\":1}"] {
            assert_eq!(Reading::from_wire(bad), None, "{bad}");
        }
        // An effect this build doesn't know: the whole reading is unreadable, never read without it.
        let odd = wire.replacen("FileDelete", "FileShred", 1);
        assert_eq!(Reading::from_wire(&odd), None);
        let other = Reading {
            v: 2,
            ..r
        };
        let signed_roots = standard();
        let signed = Folders {
            home: HOME,
            roots: &signed_roots,
            scratch: &[],
        };
        assert_eq!(accept(&other, &signed), None);
    }

    #[test]
    fn an_oversized_reading_is_neither_sent_nor_read() {
        let many = (0..400)
            .map(|i| {
                delete(path(&format!("/home/me/Documents/dev/app/target/some/long/folder/{i}"), Area::Generated, false))
            })
            .collect();
        let r = reading(many);
        assert_eq!(r.to_wire(), None, "larger than the limit: the question goes without it");
        let wire = serde_json::to_string(&r).unwrap();
        assert!(wire.len() > MAX_BYTES);
        assert_eq!(Reading::from_wire(&wire), None);
    }

    #[test]
    fn a_path_outside_the_signed_folders_is_placed_by_the_phone() {
        let signed_roots = standard();
        let signed = Folders {
            home: HOME,
            roots: &signed_roots,
            scratch: &[],
        };
        // The computer calls a folder of the home scratch (an agent wrote `scratch = ["~"]`): the phone doesn't.
        let lie = reading(vec![delete(path("/home/me/elsewhere/photos", Area::Scratch, true))]);
        let a = accept(&lie, &signed).unwrap().analysis;
        assert_eq!(a.effects, vec![delete(path("/home/me/elsewhere/photos", Area::Home, false))]);
        assert_eq!(
            outcome(&a),
            (
                Outcome::Ask {
                    hard: true
                },
                Zone::Outside,
                Level::L3
            )
        );
        // The same for the system, a root itself, the folder above one, and a path with `..` left in it.
        for (p, want) in [
            ("/etc/passwd", Area::System),
            ("/home/me/Documents/dev", Area::Source),
            ("/home/me/Documents", Area::Home),
            ("/home/me/Documents/dev/app/../../../.ssh", Area::Home),
            ("relative/path", Area::Home),
        ] {
            let a = accept(&reading(vec![delete(path(p, Area::Generated, true))]), &signed).unwrap().analysis;
            assert_eq!(a.effects, vec![delete(path(p, want, false))], "{p}");
            assert_eq!(a.level, Level::L3, "{p}");
        }
    }

    #[test]
    fn inside_the_signed_folders_the_computers_finer_word_is_kept() {
        let signed_roots = standard();
        let signed_scratch = vec!["/home/me/run".to_owned()];
        let signed = Folders {
            home: HOME,
            roots: &signed_roots,
            scratch: &signed_scratch,
        };
        let placed = |p: &str, area: Area, recoverable: bool| {
            let a = accept(&reading(vec![delete(path(p, area, recoverable))]), &signed).unwrap().analysis;
            let Effect::FileDelete {
                paths,
                ..
            } = &a.effects[0]
            else {
                panic!("a delete")
            };
            (paths[0].area.clone(), paths[0].recoverable, a.level)
        };
        // Its own scratch folder and a worktree's build output, strictly inside a signed root.
        assert_eq!(placed("/home/me/Documents/dev/tmp/run-1", Area::Scratch, false), (Area::Scratch, false, Level::L0));
        assert_eq!(
            placed("/home/me/Documents/dev/app/.claude/worktrees/a/out", Area::Generated, false),
            (Area::Generated, false, Level::L0)
        );
        // A tracked file of a project: git brings it back. The project's own folder, or a root's child: never.
        assert_eq!(placed("/home/me/Documents/dev/app/src", Area::Source, true), (Area::Source, true, Level::L1));
        assert_eq!(placed("/home/me/Documents/dev/app", Area::Source, true), (Area::Source, false, Level::L3));
        // The built-in scratch folders and a signed one.
        assert_eq!(placed("/tmp/x", Area::Scratch, false), (Area::Scratch, false, Level::L0));
        assert_eq!(placed("/home/me/.cache/pip", Area::Scratch, false), (Area::Scratch, false, Level::L0));
        assert_eq!(placed("/home/me/run/1", Area::Scratch, false), (Area::Scratch, false, Level::L0));
        // A stricter word is kept: the phone would call this build output.
        assert_eq!(placed("/home/me/Documents/dev/app/target", Area::Source, false), (Area::Source, false, Level::L3));
        assert_eq!(placed("/home/me/Documents/dev/app/target", Area::Home, false), (Area::Home, false, Level::L3));
    }

    #[test]
    fn not_known_and_another_computer_are_never_looser_than_the_phones_placing() {
        let signed_roots = standard();
        let signed = Folders {
            home: HOME,
            roots: &signed_roots,
            scratch: &[],
        };
        let write = |p: &str, area: Area| {
            let r = reading(vec![Effect::FileWrite {
                path: path(p, area, false),
            }]);
            let a = accept(&r, &signed).unwrap().analysis;
            outcome(&a)
        };
        // A write "somewhere not known" is recoverable and Mine; in the home folder it is neither.
        assert_eq!(write("$DIR/x", Area::Unknown), (Outcome::Auto, Zone::Mine, Level::L1));
        assert_eq!(
            write("/home/me/.bashrc", Area::Unknown),
            (
                Outcome::Ask {
                    hard: true
                },
                Zone::Outside,
                Level::L3
            )
        );
        assert_eq!(write("/home/me/Documents/dev/app/x", Area::Unknown), (Outcome::Auto, Zone::Mine, Level::L1));
        // Another computer's path keeps that: the worst level, the host's zone.
        let far = Area::Remote("box.example.com".to_owned());
        assert_eq!(write("/tmp/x", far.clone()).2, Level::L3);
        assert_eq!(write("/home/me/Documents/dev/app/target/x", far).2, Level::L3);
    }

    #[test]
    fn a_written_level_or_floor_is_never_used() {
        let signed_roots = standard();
        let signed = Folders {
            home: HOME,
            roots: &signed_roots,
            scratch: &[],
        };
        let secret = Effect::FileRead {
            path: path("/home/me/.ssh/id_ed25519", Area::Home, false),
            kind: FileKind::Secret(SecretKind::PrivateKey),
        };
        let publish = Effect::Publish {
            target: "crates.io".to_owned(),
        };
        let money = Effect::Money {
            amount: None,
            merchant: None,
        };
        let home_delete = delete(path("/home/me/photos", Area::Home, false));
        for (e, floor) in [
            (secret, Some(Floor::SecretRead)),
            (publish, Some(Floor::Publish)),
            (money, Some(Floor::Money)),
            (home_delete, None),
        ] {
            let mut r = reading(vec![e]);
            // What an agent would write: nothing to see here.
            r.analysis.level = Level::L0;
            r.analysis.floor = Vec::new();
            let wire = r.to_wire().unwrap();
            assert!(wire.contains("\"level\":\"L0\"") && wire.contains("\"floor\":[]"), "{wire}");
            let a = accept(&Reading::from_wire(&wire).unwrap(), &signed).unwrap().analysis;
            assert_eq!(a.level, Level::L3, "{a:?}");
            assert_eq!(a.floor, floor.into_iter().collect::<Vec<_>>(), "{a:?}");
            assert!(
                matches!(
                    outcome(&a).0,
                    Outcome::Ask {
                        hard: true
                    }
                ),
                "{a:?}"
            );
        }
        // Unreadable stays unreadable, whatever level is written.
        let mut r = reading(Vec::new());
        r.analysis.certainty = Certainty::None;
        r.analysis.floor = Vec::new();
        let a = accept(&r, &signed).unwrap().analysis;
        assert_eq!(a.floor, vec![Floor::Unreadable]);
    }

    #[test]
    fn a_workspace_the_phone_did_not_sign_makes_the_reading_unusable() {
        let signed_roots = standard();
        let signed_scratch = vec!["/home/me/run".to_owned()];
        let signed = Folders {
            home: HOME,
            roots: &signed_roots,
            scratch: &signed_scratch,
        };
        let with = |roots: &[&str], scratch: &[&str]| Reading {
            roots: roots.iter().map(|s| (*s).to_owned()).collect(),
            scratch: scratch.iter().map(|s| (*s).to_owned()).collect(),
            // A script the engine took for the project's own because of the workspace it was given: no path says so.
            ..reading(vec![Effect::Exec {
                program: "deploy.sh".to_owned(),
                source: crate::effects::CodeSource::Workspace,
            }])
        };
        assert!(accept(&with(&["/home/me/Documents/dev", "/home/me/src/group"], &[]), &signed).is_some());
        assert!(accept(&with(&[], &["/home/me/run", "/home/me/Documents/dev/tmp"]), &signed).is_some());
        for (roots, scratch) in [
            (&["/"][..], &[][..]),
            (&["/home/me"], &[]),
            (&["/home/me/elsewhere"], &[]),
            (&["/home/me/Documents"], &[]),
            (&["/home/me/Documents/dev/../../.ssh"], &[]),
            (&["~/Documents/dev"], &[]),
            (&["Documents/dev"], &[]),
            (&[], &["/home/me"]),
            (&[], &["/home/me/Documents/dev"]),
            (&[], &["/home/me/elsewhere"]),
            (&[], &["/"]),
        ] {
            assert_eq!(accept(&with(roots, scratch), &signed), None, "{roots:?} {scratch:?}");
        }
        // Another home folder than the one the phone knows for that computer.
        let other = Reading {
            home: "/home/other".to_owned(),
            ..with(&[], &[])
        };
        assert_eq!(accept(&other, &signed), None);
    }

    #[test]
    fn without_a_known_home_only_absolute_signed_folders_count() {
        // Never proposed: the phone knows no home for that computer, so `~/Documents/dev` names no folder.
        let signed_roots = standard();
        let unknown = Folders {
            home: "",
            roots: &signed_roots,
            scratch: &[],
        };
        assert_eq!(accept(&reading(Vec::new()), &unknown), None, "its roots can't be checked");
        let bare = |effects| Reading {
            roots: Vec::new(),
            ..reading(effects)
        };
        // Read with no workspace at all: the built-in scratch folders count, everything else is the home folder.
        let a = accept(&bare(vec![delete(path("/tmp/x", Area::Scratch, false))]), &unknown).unwrap().analysis;
        assert_eq!(a.level, Level::L0);
        let a = accept(&bare(vec![delete(path("/home/me/Documents/dev/app/target", Area::Generated, true))]), &unknown)
            .unwrap()
            .analysis;
        assert_eq!(a.effects, vec![delete(path("/home/me/Documents/dev/app/target", Area::Home, false))]);
        assert_eq!(a.level, Level::L3);
    }

    #[test]
    fn a_net_lowers_only_what_the_phone_places_where_a_net_can() {
        let signed_roots = standard();
        let signed = Folders {
            home: HOME,
            roots: &signed_roots,
            scratch: &[],
        };
        let source = "/home/me/Documents/dev/app/notes";
        let mut r = reading(vec![delete(path(source, Area::Source, false))]);
        r.netted = Some(vec![source.to_owned()]);
        let got = accept(&r, &signed).unwrap();
        assert_eq!((got.analysis.level, got.netted.as_ref().map(|n| n.level)), (Level::L3, Some(Level::L1)));
        assert_eq!(got.netted.as_ref().map(|n| &n.effects), Some(&got.analysis.effects), "the same effects");
        // A level the computer wrote for the netted request plays no part: only the paths it says the net keeps.
        let mut lie = reading(vec![delete(path("/etc/nginx", Area::Source, false))]);
        lie.netted = Some(vec!["/etc/nginx".to_owned()]);
        let got = accept(&lie, &signed).unwrap();
        assert_eq!(got.netted.map(|n| n.level), Some(Level::L3), "the system's files are never lowered by a net");
        // What couldn't be read whole stays at least recoverable, and its floor stays.
        let mut partial = reading(vec![delete(path(source, Area::Source, false))]);
        partial.analysis.certainty = Certainty::None;
        partial.netted = Some(vec![source.to_owned()]);
        let got = accept(&partial, &signed).unwrap().netted.unwrap();
        assert_eq!((got.level, got.floor), (Level::L1, vec![Floor::Unreadable]));
    }

    #[test]
    fn about_binds_the_action_the_folder_and_the_net() {
        let base = about(&Asked::Command("rm -rf build"), Some("/home/me/app"), None);
        assert_eq!(base.len(), 64);
        assert_eq!(base, about(&Asked::Command("rm -rf build"), Some("/home/me/app"), None));
        let others = [
            about(&Asked::Command("rm -rf build "), Some("/home/me/app"), None),
            about(&Asked::Command("rm -rf build"), Some("/home/me"), None),
            about(&Asked::Command("rm -rf build"), None, None),
            about(&Asked::Command("rm -rf build"), Some("/home/me/app"), Some("reins net run -- rm -rf build")),
            // Parts can't slide into each other: each has its length.
            about(&Asked::Command("rm -rf build\n\nIn: /home/me/app"), None, None),
            about(
                &Asked::Files {
                    write: false,
                    paths: &["rm -rf build"],
                },
                Some("/home/me/app"),
                None,
            ),
        ];
        for (i, o) in others.iter().enumerate() {
            assert_ne!(*o, base, "{i}");
        }
        let files = |write, paths: &[&str]| {
            about(
                &Asked::Files {
                    write,
                    paths,
                },
                None,
                None,
            )
        };
        assert_ne!(files(false, &[".env"]), files(true, &[".env"]));
        assert_ne!(files(false, &["a", "b"]), files(false, &["a\nb"]));
        assert_ne!(files(false, &["a", "b"]), files(false, &["b", "a"]));
    }

    #[test]
    fn the_computers_own_settings_only_tighten() {
        use crate::desktop_policy::tighten_with_local;
        use crate::matrix::Cell;
        let mut snap = Snapshot::default();
        snap.facts.set(FactKind::Owner, "dkat", Zone::Mine);
        let mut loose = Matrix::default();
        for z in Zone::ALL {
            for l in [Level::L0, Level::L1, Level::L2, Level::L3] {
                loose.set(z, l, Cell::Auto);
            }
        }
        let mut facts = Facts::default();
        facts.set(FactKind::Owner, "evil", Zone::Mine);
        facts.set(FactKind::Owner, "dkat", Zone::Work);
        let before = snap.clone();
        tighten_with_local(&mut snap, &facts, &loose);
        assert_eq!(snap.matrix, before.matrix, "an all-Auto matrix loosens nothing");
        assert_eq!(snap.facts.get(FactKind::Owner, "evil"), None, "a fact the phone doesn't have is not added");
        assert_eq!(snap.facts.get(FactKind::Owner, "dkat"), Some(Zone::Work), "a stricter zone counts");
        let mut strict = Matrix::default();
        strict.set(Zone::Mine, Level::L0, Cell::Ask);
        tighten_with_local(&mut snap, &Facts::default(), &strict);
        assert_eq!(snap.matrix.cell(Zone::Mine, Level::L0), Cell::Ask);
    }
}
