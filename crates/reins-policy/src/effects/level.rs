//! Levels, the floor, and an analysis as a whole (01 §2.3, §2.4; 08 R2, R4).

use serde::{Deserialize, Serialize};

use super::model::{Area, Certainty, CodeSource, Effect, FileKind, Floor, InfraChange, Level, NetMethod, PathRef};

/// Recipients from which a message is mass messaging (01 §2.3, configurable later).
pub const MASS_MESSAGE: usize = 10;
/// The most lines an analysis describes.
pub const MAX_LINES: usize = 12;

/// The level of a write or a delete at `p` (01 §2.5; 08 R2: without an enforced net, a delete in workspace source is
/// L1 only when git can bring it back).
pub(super) fn path_level(p: &PathRef, delete: bool) -> Level {
    match &p.area {
        Area::Generated | Area::Scratch => Level::L0,
        Area::Source if !delete || p.recoverable => Level::L1,
        // Somewhere not known: a write is what it is anywhere, a delete may take anything (01 §2.4).
        Area::Unknown if !delete => Level::L1,
        Area::Source | Area::Unknown | Area::Home | Area::System | Area::Remote(_) => Level::L3,
    }
}

/// One effect's level.
#[must_use]
pub fn level_of(e: &Effect) -> Level {
    match e {
        Effect::FileRead {
            path,
            kind,
        } => match kind {
            FileKind::Env if path.area.in_workspace() || path.area == Area::Unknown => Level::L1,
            FileKind::Secret(_) | FileKind::Env => Level::L3,
            FileKind::Other => Level::L0,
        },
        Effect::FileWrite {
            path,
        } => path_level(path, false),
        Effect::FileDelete {
            paths,
            ..
        } => paths.iter().map(|p| path_level(p, true)).max().unwrap_or(Level::L0),
        Effect::GitPush {
            refname,
            default_branch,
            rewrites,
            ..
        } => {
            let default = default_branch.unwrap_or(true);
            let rewrites = rewrites.unwrap_or(true);
            if refname.starts_with("refs/tags/") {
                if rewrites {
                    Level::L3
                } else {
                    Level::L2
                }
            } else {
                match (default, rewrites) {
                    (true, true) => Level::L3,
                    (true, false) => Level::L2,
                    (false, true) => Level::L1,
                    (false, false) => Level::L0,
                }
            }
        }
        Effect::GitRefDelete {
            default_branch,
            ..
        } => {
            if default_branch.unwrap_or(true) {
                Level::L3
            } else {
                Level::L1
            }
        }
        Effect::Message {
            to,
            ..
        } => {
            if to.len() >= MASS_MESSAGE {
                Level::L3
            } else {
                Level::L2
            }
        }
        Effect::Publish {
            ..
        }
        | Effect::Permission {
            ..
        }
        | Effect::Money {
            ..
        }
        | Effect::SecretOut {
            ..
        } => Level::L3,
        Effect::SecretUse {
            local_only,
            ..
        } => {
            if *local_only {
                Level::L1
            } else {
                Level::L3
            }
        }
        Effect::Network {
            host,
            method,
        } => match method {
            NetMethod::Read => Level::L0,
            _ if local_host(host) => Level::L0,
            NetMethod::Write | NetMethod::Connect => Level::L2,
            NetMethod::UploadExecutable => Level::L3,
        },
        Effect::Infra {
            local,
            change,
            ..
        } => match (local, change) {
            (true, _) => Level::L1,
            (false, InfraChange::Apply | InfraChange::Destroy) => Level::L3,
        },
        Effect::Exec {
            source,
            ..
        } => match source {
            CodeSource::Downloaded => Level::L3,
            CodeSource::Workspace | CodeSource::Toolchain => Level::L0,
            CodeSource::Ci => Level::L2,
            CodeSource::Generated | CodeSource::Unknown => Level::L1,
        },
        Effect::DataRead {
            ..
        } => Level::L0,
        Effect::DataWrite {
            lost,
            ..
        } => {
            if *lost {
                Level::L1
            } else {
                Level::L0
            }
        }
        // The worst plausible effect in the workspace (01 §2.4).
        Effect::Unknown {
            ..
        } => Level::L1,
    }
}

/// A host on this computer.
#[must_use]
pub fn local_host(host: &str) -> bool {
    let h = host.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    h == "localhost" || h.ends_with(".localhost") || h == "::1" || h.starts_with("127.") || h == "0.0.0.0"
}

/// The always-ask reason an effect carries, if any (08 R4).
#[must_use]
pub fn floor_of(e: &Effect) -> Option<Floor> {
    match e {
        Effect::FileRead {
            kind: FileKind::Secret(_),
            ..
        } => Some(Floor::SecretRead),
        Effect::SecretOut {
            destination,
            ..
        } => Some(if destination == AGENT {
            Floor::SecretRelease
        } else {
            Floor::SecretOut
        }),
        Effect::Publish {
            ..
        } => Some(Floor::Publish),
        Effect::Permission {
            ..
        } => Some(Floor::Permission),
        Effect::Money {
            ..
        } => Some(Floor::Money),
        Effect::Message {
            to,
            ..
        } if to.len() >= MASS_MESSAGE => Some(Floor::MassMessage),
        Effect::Network {
            host,
            method: NetMethod::UploadExecutable,
        } if !local_host(host) => Some(Floor::ExecutableUpload),
        Effect::DataRead {
            service,
            resource,
        } if service == "reins" && resource == ACCOUNTS => Some(Floor::AccountListing),
        _ => None,
    }
}

/// The destination of a vault secret handed to the AI that asked.
pub const AGENT: &str = "the AI agent";
/// The resource of `reins_list_accounts`.
pub const ACCOUNTS: &str = "accounts and their addresses";

/// What a request does, as the engine read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Analysis {
    pub effects: Vec<Effect>,
    pub level: Level,
    pub certainty: Certainty,
    pub floor: Vec<Floor>,
    /// What could not be read or resolved, in words ("the value of $DIR").
    pub unresolved: Vec<String>,
    /// The guard groups the request matches: labels for people, never triggers (01 §2.2).
    pub labels: Vec<&'static str>,
}

impl Default for Analysis {
    fn default() -> Self {
        Self::new(Vec::new(), Certainty::Full, Vec::new())
    }
}

impl Analysis {
    /// An analysis of `effects`: its level is the worst effect's, the floor every effect's, plus `Unreadable` for
    /// certainty `None`. Duplicate effects are dropped.
    #[must_use]
    pub fn new(effects: Vec<Effect>, certainty: Certainty, unresolved: Vec<String>) -> Self {
        let mut unique: Vec<Effect> = Vec::new();
        for e in effects {
            if !unique.contains(&e) {
                unique.push(e);
            }
        }
        let mut certainty = certainty;
        let mut unresolved = unresolved;
        // A branch that may be the default: taken as it, and not read with certainty.
        if unique.iter().any(|e| {
            matches!(
                e,
                Effect::GitPush {
                    default_branch: None,
                    ..
                } | Effect::GitRefDelete {
                    default_branch: None,
                    ..
                }
            )
        }) {
            certainty = Certainty::None;
            unresolved.push("whether the branch is the default one".to_owned());
        }
        let level = unique.iter().map(level_of).max().unwrap_or(Level::L0);
        let mut floor: Vec<Floor> = unique.iter().filter_map(floor_of).collect();
        if certainty == Certainty::None {
            floor.push(Floor::Unreadable);
        }
        floor.sort();
        floor.dedup();
        let mut unresolved_unique: Vec<String> = Vec::new();
        for u in unresolved {
            if !unresolved_unique.contains(&u) {
                unresolved_unique.push(u);
            }
        }
        // Partial certainty is the worst plausible effect in the workspace: at least L1 (01 §2.4).
        let level = if certainty == Certainty::Full {
            level
        } else {
            level.max(Level::L1)
        };
        Self {
            effects: unique,
            level,
            certainty,
            floor,
            unresolved: unresolved_unique,
            labels: Vec::new(),
        }
    }

    /// A person must look at it whatever the policy: the floor, certainty `None`, or L3.
    #[must_use]
    pub fn needs_human(&self) -> bool {
        !self.floor.is_empty() || self.certainty == Certainty::None || self.level == Level::L3
    }

    /// One plain line per effect (at most [`MAX_LINES`]); file effects of one kind and area are grouped.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        super::describe::lines(&self.effects, MAX_LINES)
    }

    /// What the activity log keeps.
    #[must_use]
    pub fn summary(&self) -> Summary {
        Summary {
            level: self.level,
            certainty: self.certainty,
            lines: self.lines().iter().map(|l| bounded(l)).collect(),
            unresolved: self.unresolved.iter().take(MAX_LINES).map(|l| bounded(l)).collect(),
            floor: self.floor.clone(),
            labels: self.labels.iter().map(|l| (*l).to_owned()).collect(),
        }
    }
}

/// The longest line a summary keeps, in characters.
pub const MAX_LINE: usize = 200;

/// `line`, cut to [`MAX_LINE`] characters.
fn bounded(line: &str) -> String {
    if line.chars().count() <= MAX_LINE {
        return line.to_owned();
    }
    let mut cut: String = line.chars().take(MAX_LINE - 1).collect();
    cut.push('\u{2026}');
    cut
}

/// An analysis as the activity log keeps and shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    pub level: Level,
    pub certainty: Certainty,
    pub lines: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub floor: Vec<Floor>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::super::model::{Channel, SecretKind};
    use super::*;

    fn path(area: Area, recoverable: bool) -> PathRef {
        PathRef {
            path: "/x".to_owned(),
            area,
            recoverable,
        }
    }

    fn delete(area: Area, recoverable: bool) -> Effect {
        Effect::FileDelete {
            paths: vec![path(area, recoverable)],
            recursive: true,
        }
    }

    fn push(default: bool, rewrites: bool) -> Effect {
        Effect::GitPush {
            repo: "github.com/dkat/reins".to_owned(),
            refname: "refs/heads/x".to_owned(),
            default_branch: Some(default),
            rewrites: Some(rewrites),
            commits: 1,
        }
    }

    #[test]
    fn levels_follow_01_2_3() {
        assert_eq!(level_of(&delete(Area::Generated, false)), Level::L0);
        assert_eq!(level_of(&delete(Area::Scratch, false)), Level::L0);
        assert_eq!(level_of(&delete(Area::Source, true)), Level::L1);
        assert_eq!(level_of(&delete(Area::Source, false)), Level::L3, "08 R2: no net, not in git");
        assert_eq!(level_of(&delete(Area::Home, true)), Level::L3);
        assert_eq!(level_of(&delete(Area::System, false)), Level::L3);
        assert_eq!(
            level_of(&Effect::FileWrite {
                path: path(Area::Source, false)
            }),
            Level::L1
        );
        assert_eq!(level_of(&push(false, false)), Level::L0);
        assert_eq!(level_of(&push(false, true)), Level::L1);
        assert_eq!(level_of(&push(true, false)), Level::L2);
        assert_eq!(level_of(&push(true, true)), Level::L3);
        let message = |n: usize| Effect::Message {
            channel: Channel::Email,
            to: (0..n).map(|i| format!("p{i}@example.com")).collect(),
            attachments: 0,
        };
        assert_eq!(level_of(&message(2)), Level::L2);
        assert_eq!(level_of(&message(10)), Level::L3);
        assert_eq!(floor_of(&message(10)), Some(Floor::MassMessage));
        let read = |kind| Effect::FileRead {
            path: path(Area::Home, false),
            kind,
        };
        assert_eq!(floor_of(&read(FileKind::Secret(SecretKind::PrivateKey))), Some(Floor::SecretRead));
        let env = Effect::FileRead {
            path: path(Area::Source, false),
            kind: FileKind::Env,
        };
        assert_eq!((level_of(&env), floor_of(&env)), (Level::L1, None), "08 R4: .env by zone and level");
        assert_eq!(level_of(&read(FileKind::Other)), Level::L0);
        assert_eq!(
            level_of(&Effect::Network {
                host: "127.0.0.1".to_owned(),
                method: NetMethod::Write
            }),
            Level::L0
        );
    }

    #[test]
    fn analysis_takes_the_worst() {
        let a = Analysis::new(vec![push(false, false), delete(Area::Home, false)], Certainty::Full, vec![]);
        assert_eq!(a.level, Level::L3);
        assert!(a.needs_human());
        let partial = Analysis::new(vec![], Certainty::Partial, vec!["the value of $D".to_owned()]);
        assert_eq!(partial.level, Level::L1, "unresolved: the worst plausible effect in the workspace");
        assert!(!partial.needs_human());
        let none = Analysis::new(vec![], Certainty::None, vec![]);
        assert_eq!(none.floor, vec![Floor::Unreadable]);
        assert!(none.needs_human());
        let twice = Analysis::new(vec![push(false, false), push(false, false)], Certainty::Full, vec![]);
        assert_eq!(twice.effects.len(), 1);
    }
}
