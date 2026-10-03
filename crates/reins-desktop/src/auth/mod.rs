//! Who allows a git read or push, and where the credential comes from. Two authorizers: the phone through the Rewarden
//! server ([`rewarden::RewardenAuthorizer`]) and, without a phone, the local policy with a local token for the host
//! ([`local::LocalAuthorizer`]). The proxy only sees this trait.

pub mod local;
pub mod policy;
pub mod prompt;
pub mod rewarden;

use data_encoding::BASE64;
use rewarden_proto::desktop::PushSummary;
use zeroize::Zeroizing;

/// A repository on a git host, as named in the proxy path (`/github.com/owner/name.git/...`,
/// `/gitlab.com/group/subgroup/name.git/...`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Repo {
    /// The configured host (`gitlab.com`).
    pub host: String,
    /// The host's service ([`crate::config::GIT_SERVICES`]): which of the phone's tools are asked.
    pub service: String,
    /// `owner/name`, on GitLab `group/subgroup/name`; without `.git`.
    pub path: String,
}

impl Repo {
    #[must_use]
    pub fn new(host: &str, service: &str, path: &str) -> Self {
        Self {
            host: host.to_owned(),
            service: service.to_owned(),
            path: path.to_owned(),
        }
    }

    /// The repository path: the resource the phone's permissions for the host use.
    #[must_use]
    pub fn full_name(&self) -> String {
        self.path.clone()
    }

    /// `host/path`, for people.
    #[must_use]
    pub fn label(&self) -> String {
        format!("{}/{}", self.host, self.path)
    }

    /// How local messages name it: the path on GitHub (as they always have), `host/path` elsewhere.
    #[must_use]
    pub fn short(&self) -> String {
        if self.service == "github" {
            self.path.clone()
        } else {
            self.label()
        }
    }
}

/// A token for one repository, held in memory only.
#[derive(Clone)]
pub struct Credential {
    pub username: String,
    pub token: Zeroizing<String>,
    /// Unix seconds; the credential is not used after it.
    pub expires_at: i64,
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credential")
            .field("username", &self.username)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

impl Credential {
    /// The `Authorization` header value (HTTP basic, as git and the GitHub API accept for tokens).
    #[must_use]
    pub fn authorization(&self) -> Zeroizing<String> {
        let pair = Zeroizing::new(format!("{}:{}", self.username, self.token.as_str()));
        Zeroizing::new(format!("Basic {}", BASE64.encode(pair.as_bytes())))
    }

    #[must_use]
    pub fn is_live(&self, now: i64) -> bool {
        now < self.expires_at
    }
}

/// Why a read or a push does not go ahead. The text is shown to the person (or agent) running git.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Said no (the user on the phone, or the policy).
    Denied(String),
    /// Still waiting for the user; git may be run again, and a later approval of the same push still counts.
    Waiting(String),
    /// Cannot be decided now (offline phone, not logged in, no token, an error).
    Unavailable(String),
}

impl Refusal {
    #[must_use]
    pub fn message(&self) -> &str {
        match self {
            Self::Denied(m) | Self::Waiting(m) | Self::Unavailable(m) => m,
        }
    }
}

#[async_trait::async_trait]
pub trait Authorizer: Send + Sync {
    /// Read access to `repo`: a clone or fetch of a repository the anonymous request could not get, and the ref list
    /// before a push. Implementations cache the credential until it expires.
    async fn read(&self, repo: &Repo) -> Result<Credential, Refusal>;

    /// Approval for exactly this push (`digest` from [`rewarden_proto::desktop::push_digest`]); the credential is used
    /// for this push only.
    async fn push(&self, repo: &Repo, summary: &PushSummary, digest: &str) -> Result<Credential, Refusal>;

    /// Who decides, for `rewarden status` ("your phone via rewarden.example.com", "the local policy").
    fn describe(&self) -> String;

    /// Where the person running git should look while an answer is awaited.
    fn waiting_hint(&self) -> String {
        "waiting for approval".to_owned()
    }
}
