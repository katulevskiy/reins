//! The typed consequences of a request (`docs/design/decisions-v2/01-decision-engine.md` §2.1).
//!
//! References are plain strings with the facts the levels need (the area of a path, whether a branch is the default).
//! The context graph (zones: Mine, Friends, Work, Outside) is not here: it is resolved later, from verified facts,
//! never from these strings alone.

use serde::{Deserialize, Serialize};

/// Where a local path is, as the workspace sees it (01 §2.5).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Area {
    /// Build output and caches inside the workspace (`target/`, `node_modules/`, ...).
    Generated,
    /// The OS temp directory, `~/.cache`, configured scratch roots, a fresh `mktemp` directory.
    Scratch,
    /// Anything else inside the workspace: a project's own files.
    Source,
    /// The home folder outside the workspace.
    Home,
    /// Anything outside the home folder.
    System,
    /// A path on another computer (over `ssh`, `scp`, `rsync`).
    Remote(String),
    /// Not known (a path computed at run time).
    Unknown,
}

impl Area {
    /// Inside the workspace (its source, its output, or scratch).
    #[must_use]
    pub fn in_workspace(&self) -> bool {
        matches!(self, Self::Generated | Self::Scratch | Self::Source)
    }
}

/// A local (or remote) path and what is known about it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PathRef {
    /// Normalized and absolute when it could be (`/home/me/Documents/dev/app/target`), as written otherwise.
    pub path: String,
    pub area: Area,
    /// Recoverable from git: inside a known worktree, or reported tracked by the daemon (08 R2).
    pub recoverable: bool,
}

/// What kind of secret a file holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SecretKind {
    PrivateKey,
    CloudCredential,
    Keystore,
    Token,
    /// Reins's own sign-in and keys on the computer.
    Reins,
}

/// What kind of file a read touches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileKind {
    Secret(SecretKind),
    /// A `.env` file: judged by zone and level, not the floor (08 R4).
    Env,
    Other,
}

/// How a message is sent.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Channel {
    Email,
    Telegram,
    Sms,
    /// A calendar invitation.
    Calendar,
    /// A pull request, issue or comment on a repository: its people are notified.
    GitHub,
    /// Another service (an MCP server's chat).
    Other(String),
}

/// What a network request does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NetMethod {
    /// Fetches something (a GET, a clone).
    Read,
    /// Sends data (a POST, an upload of a file).
    Write,
    /// Uploads a file that may be run (a script, a binary, an archive).
    UploadExecutable,
    /// An interactive or opaque connection (`ssh host`, `nc`).
    Connect,
}

/// What an infrastructure change does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InfraChange {
    /// Applies a plan or manifests (`terraform apply`, `kubectl apply`, DDL that creates or alters).
    Apply,
    /// Destroys or drops (`terraform destroy`, `kubectl delete`, `DROP TABLE`, deleting a repository).
    Destroy,
}

/// Where a program that runs comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CodeSource {
    /// Fetched from the network and run (`curl … | sh`).
    Downloaded,
    /// Made at run time and run, unseen (`base64 -d | sh`, a script written elsewhere).
    Generated,
    /// The project's own build and test entry points (`./gradlew`, `make`, `cargo test`).
    Workspace,
    /// An installed developer tool doing its usual job (`adb`, `ffmpeg`, `xcodebuild`).
    Toolchain,
    /// A continuous-integration run (a workflow dispatched, a run started again).
    Ci,
    /// A program this engine has no model of.
    Unknown,
}

/// One consequence of a request.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Effect {
    FileRead {
        path: PathRef,
        kind: FileKind,
    },
    FileWrite {
        path: PathRef,
    },
    FileDelete {
        paths: Vec<PathRef>,
        recursive: bool,
    },
    /// A push. `default_branch` and `rewrites` are `None` when not known (taken as the worse).
    GitPush {
        repo: String,
        refname: String,
        default_branch: Option<bool>,
        rewrites: Option<bool>,
        commits: u32,
    },
    GitRefDelete {
        repo: String,
        refname: String,
        default_branch: Option<bool>,
    },
    Message {
        channel: Channel,
        /// Recipients as the call names them.
        to: Vec<String>,
        attachments: u32,
    },
    /// Public for everyone: a package, a release, a public repository, a public link.
    Publish {
        target: String,
    },
    /// Who can do what: collaborators, keys, branch protection, secrets of CI.
    Permission {
        resource: String,
        change: String,
    },
    Money {
        amount: Option<String>,
        merchant: Option<String>,
    },
    /// A secret leaves this device or the phone.
    SecretOut {
        secret: String,
        destination: String,
    },
    /// A secret is used where it stays (a local signing step).
    SecretUse {
        secret: String,
        local_only: bool,
    },
    Network {
        host: String,
        method: NetMethod,
    },
    Infra {
        target: String,
        change: InfraChange,
        /// On this computer (a local database file, a local cluster).
        local: bool,
    },
    Exec {
        program: String,
        source: CodeSource,
    },
    /// Reads from a connected service (not in 01 §2.1: connector reads have no effect type there).
    DataRead {
        service: String,
        resource: String,
    },
    /// A change in a connected service that is not a message, a permission or money (not in 01 §2.1): a draft, a
    /// label, a calendar entry without guests, a vault item. `lost`: something is removed or overwritten (it can
    /// still be restored from the service's own trash or history).
    DataWrite {
        service: String,
        what: String,
        lost: bool,
    },
    /// Could not be worked out; `reason` says what.
    Unknown {
        reason: String,
    },
}

/// A request's consequence level (01 §2.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum Level {
    /// Undoable: nothing is lost or seen by anyone else.
    L0,
    /// Recoverable: something is lost, but a way back exists.
    L1,
    /// Reaches people.
    L2,
    /// Irreversible or far-reaching.
    L3,
}

impl Level {
    /// The words a person reads.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::L0 => "Undoable",
            Self::L1 => "Recoverable",
            Self::L2 => "Reaches people",
            Self::L3 => "Can't be undone",
        }
    }
}

/// How well the engine read a request (01 §2.4). Ordered from best to worst.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum Certainty {
    /// Every effect is known exactly.
    Full,
    /// Something is unresolved: assumed the worst plausible effect in the workspace.
    Partial,
    /// It could not be read at all: always asked.
    None,
}

impl Certainty {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Full => "Full",
            Self::Partial => "Partial",
            Self::None => "None",
        }
    }
}

/// The always-ask floor (08 R4): hard reviews whatever the level or the matrix says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum Floor {
    /// A private key, a cloud credential, a keystore or a token file is read into the agent's context.
    SecretRead,
    /// A secret leaves for a destination.
    SecretOut,
    /// A vault secret is released to an AI.
    SecretRelease,
    /// The list of accounts with their addresses.
    AccountListing,
    /// An upload of something that may be run.
    ExecutableUpload,
    Money,
    Publish,
    Permission,
    /// Ten or more recipients.
    MassMessage,
    /// Certainty `None`.
    Unreadable,
}

impl Floor {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::SecretRead => "Reads a secret",
            Self::SecretOut => "A secret leaves",
            Self::SecretRelease => "Releases a vault secret",
            Self::AccountListing => "Lists your accounts",
            Self::ExecutableUpload => "Uploads something that runs",
            Self::Money => "Spends money",
            Self::Publish => "Publishes",
            Self::Permission => "Changes who can do what",
            Self::MassMessage => "Messages many people",
            Self::Unreadable => "Couldn't be read",
        }
    }
}

/// Reads a value as a newer build may have written it: a name this build doesn't know reads as `$worst`.
macro_rules! lenient {
    ($ty:ident, $worst:ident, [$($v:ident),+ $(,)?]) => {
        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let name = String::deserialize(d)?;
                Ok(match name.as_str() {
                    $(stringify!($v) => Self::$v,)+
                    _ => Self::$worst,
                })
            }
        }
    };
}

lenient!(Level, L3, [L0, L1, L2, L3]);
lenient!(Certainty, None, [Full, Partial, None]);
lenient!(
    Floor,
    Unreadable,
    [
        SecretRead,
        SecretOut,
        SecretRelease,
        AccountListing,
        ExecutableUpload,
        Money,
        Publish,
        Permission,
        MassMessage,
        Unreadable
    ]
);
/// An effect's kind, for rules that name kinds (`docs/design/decisions-v2/02-policy-and-authority.md` §2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectKind {
    FileRead,
    FileWrite,
    FileDelete,
    GitPush,
    GitRefDelete,
    Message,
    Publish,
    Permission,
    Money,
    SecretOut,
    SecretUse,
    Network,
    Infra,
    Exec,
    DataRead,
    DataWrite,
    Unknown,
}

impl Effect {
    #[must_use]
    pub fn kind(&self) -> EffectKind {
        match self {
            Self::FileRead {
                ..
            } => EffectKind::FileRead,
            Self::FileWrite {
                ..
            } => EffectKind::FileWrite,
            Self::FileDelete {
                ..
            } => EffectKind::FileDelete,
            Self::GitPush {
                ..
            } => EffectKind::GitPush,
            Self::GitRefDelete {
                ..
            } => EffectKind::GitRefDelete,
            Self::Message {
                ..
            } => EffectKind::Message,
            Self::Publish {
                ..
            } => EffectKind::Publish,
            Self::Permission {
                ..
            } => EffectKind::Permission,
            Self::Money {
                ..
            } => EffectKind::Money,
            Self::SecretOut {
                ..
            } => EffectKind::SecretOut,
            Self::SecretUse {
                ..
            } => EffectKind::SecretUse,
            Self::Network {
                ..
            } => EffectKind::Network,
            Self::Infra {
                ..
            } => EffectKind::Infra,
            Self::Exec {
                ..
            } => EffectKind::Exec,
            Self::DataRead {
                ..
            } => EffectKind::DataRead,
            Self::DataWrite {
                ..
            } => EffectKind::DataWrite,
            Self::Unknown {
                ..
            } => EffectKind::Unknown,
        }
    }
}
