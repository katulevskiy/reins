//! Organizations (Decisions v2 · O1, `docs/design/decisions-v2/05-organizations.md`): what the server, the phone core
//! and the desktop agree on about a Managed tenant. Tiers and identity, the roles of 05 §3 and what each may do, the
//! admin actions that take two people, domains, device enrollment and posture (06 §6), and which profile a git remote
//! belongs to (08 R6).

use serde::{Deserialize, Serialize};

/// The tenant's plan (05 §11, 08 R11). Flags only: nothing is billed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// Google Workspace or GitHub org sign-in plus a verified domain; no SAML, no SCIM.
    Team,
    /// SAML/OIDC SSO and SCIM.
    Business,
    /// Business, dedicated or self-hosted.
    Enterprise,
}

impl Tier {
    pub const ALL: [Self; 3] = [Self::Team, Self::Business, Self::Enterprise];

    /// Whether members of a tenant on this tier may sign in with `identity`.
    #[must_use]
    pub fn allows(self, identity: IdentityKind) -> bool {
        match identity {
            IdentityKind::None | IdentityKind::GoogleWorkspace | IdentityKind::GithubOrg => true,
            IdentityKind::Sso => self != Self::Team,
        }
    }

    /// Whether the tier provisions members from a directory (SCIM).
    #[must_use]
    pub fn scim(self) -> bool {
        self != Self::Team
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Team => "team",
            Self::Business => "business",
            Self::Enterprise => "enterprise",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == s)
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Team => "Team",
            Self::Business => "Business",
            Self::Enterprise => "Enterprise",
        }
    }
}

/// How the tenant's members prove who they are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityKind {
    /// Not set up yet: nobody but the creator gets in.
    None,
    /// Google sign-in with an email in a verified domain (Team).
    GoogleWorkspace,
    /// GitHub sign-in by an active member of the tenant's GitHub organization (Team).
    GithubOrg,
    /// SAML or OIDC through the tenant's WorkOS organization (Business, Enterprise).
    Sso,
}

impl IdentityKind {
    pub const ALL: [Self; 4] = [Self::None, Self::GoogleWorkspace, Self::GithubOrg, Self::Sso];

    /// The WorkOS `authentication_method` values a sign-in through this identity has.
    #[must_use]
    pub fn workos_methods(self) -> &'static [&'static str] {
        match self {
            Self::None => &[],
            Self::GoogleWorkspace => &["GoogleOAuth"],
            Self::GithubOrg => &["GitHubOAuth"],
            Self::Sso => &["SSO"],
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::GoogleWorkspace => "google_workspace",
            Self::GithubOrg => "github_org",
            Self::Sso => "sso",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == s)
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "Not set up",
            Self::GoogleWorkspace => "Google Workspace",
            Self::GithubOrg => "GitHub organization",
            Self::Sso => "SSO (SAML or OIDC)",
        }
    }
}

/// The roles of 05 §3.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    OrgOwner,
    SecurityAdmin,
    PolicyAuthor,
    ConnectionAdmin,
    Approver,
    Auditor,
    TeamLead,
    Member,
}

impl Role {
    pub const ALL: [Self; 8] = [
        Self::OrgOwner,
        Self::SecurityAdmin,
        Self::PolicyAuthor,
        Self::ConnectionAdmin,
        Self::Approver,
        Self::Auditor,
        Self::TeamLead,
        Self::Member,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OrgOwner => "org_owner",
            Self::SecurityAdmin => "security_admin",
            Self::PolicyAuthor => "policy_author",
            Self::ConnectionAdmin => "connection_admin",
            Self::Approver => "approver",
            Self::Auditor => "auditor",
            Self::TeamLead => "team_lead",
            Self::Member => "member",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.as_str() == s)
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::OrgOwner => "Org owner",
            Self::SecurityAdmin => "Security admin",
            Self::PolicyAuthor => "Policy author",
            Self::ConnectionAdmin => "Connection admin",
            Self::Approver => "Approver",
            Self::Auditor => "Auditor",
            Self::TeamLead => "Team lead",
            Self::Member => "Member",
        }
    }

    /// The roles whose grant and removal only an Org owner may propose or approve. They are granted to people only,
    /// never to a group (a group's members come from the identity provider).
    #[must_use]
    pub fn is_admin(self) -> bool {
        matches!(self, Self::OrgOwner | Self::SecurityAdmin)
    }

    /// The roles that configure the org, its policy or its connections: acting *on* someone who holds one (removing
    /// them) is an admin action of its own ([`AdminAction::capability`]).
    #[must_use]
    pub fn is_privileged(self) -> bool {
        matches!(self, Self::OrgOwner | Self::SecurityAdmin | Self::ConnectionAdmin | Self::PolicyAuthor)
    }
}

/// What a role may do. O1 uses the first seven; the rest name what O2–O4 build.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Domains, sign-in (SSO/SCIM), tier, the two-person setting.
    SetUpOrg,
    /// Grant or remove Org owner and Security admin.
    AssignAdmins,
    /// Grant or remove the other roles.
    AssignRoles,
    /// Remove a member by hand.
    ManageMembers,
    ViewMembers,
    ViewAdminEvents,
    AuthorPolicy,
    ActivatePolicy,
    ManageConnections,
    /// Satisfy reviews routed to the role's groups.
    Approve,
}

/// Whether `role` carries `cap` (05 §3).
#[must_use]
pub fn role_grants(role: Role, cap: Capability) -> bool {
    use Capability as C;
    match role {
        Role::OrgOwner => matches!(
            cap,
            C::SetUpOrg | C::AssignAdmins | C::AssignRoles | C::ManageMembers | C::ViewMembers | C::ViewAdminEvents
        ),
        Role::SecurityAdmin => matches!(
            cap,
            C::AssignRoles
                | C::ManageMembers
                | C::ViewMembers
                | C::ViewAdminEvents
                | C::AuthorPolicy
                | C::ActivatePolicy
                | C::ManageConnections
        ),
        Role::PolicyAuthor => cap == C::AuthorPolicy,
        Role::ConnectionAdmin => matches!(cap, C::ManageConnections | C::ViewMembers),
        Role::Approver => cap == C::Approve,
        Role::Auditor => matches!(cap, C::ViewMembers | C::ViewAdminEvents),
        Role::TeamLead => matches!(cap, C::ViewMembers | C::ManageConnections),
        Role::Member => false,
    }
}

/// Whom a role is granted to: one member (their membership id) or a group (its id; SCIM groups included).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum Subject {
    Member(String),
    Group(String),
}

/// An admin action (05 §3): proposed by one holder of [`AdminAction::capability`] and, with two-person approval on,
/// applied only when another holder approves.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AdminAction {
    GrantRole {
        subject: Subject,
        role: Role,
    },
    RevokeRole {
        subject: Subject,
        role: Role,
    },
    SetIdentity {
        identity: IdentityKind,
        #[serde(default)]
        workos_org_id: Option<String>,
        #[serde(default)]
        directory_id: Option<String>,
        #[serde(default)]
        github_org: Option<String>,
    },
    SetTier {
        tier: Tier,
    },
    RemoveDomain {
        domain: String,
    },
    /// `member`: the membership id.
    RemoveMember {
        member: String,
    },
    SetTwoPerson {
        on: bool,
    },
    /// Lets the directory sync remove the members it held back: more than a backfill removes without an admin
    /// (a listing that lost most of a directory looks like a broken listing). `held`: the digest of exactly the
    /// members held, so what's approved is what was shown.
    ConfirmDirectoryRemovals {
        held: String,
    },
    /// Makes a drafted policy revision the org's active policy, signed by the org issuer key (O2). `hash` is the draft's
    /// hash, so what's approved is what was shown.
    ActivatePolicy {
        revision: u64,
        hash: String,
        /// The simulation report this activation rests on (O4): its hash. Needed once a policy is active.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        report: Option<String>,
    },
    /// How long request bodies are kept: 0, 30 or 365 days (06 §4).
    SetRetention {
        days: u32,
    },
    /// Registers an executor for this org with its Ed25519 public key (base64url).
    RegisterExecutor {
        executor_id: String,
        mode: ExecutorMode,
        public_key: String,
    },
    RemoveExecutor {
        executor_id: String,
    },
    /// Binds the org's policy repo (`owner/name` on GitHub, O4); an empty `repo` unbinds it. The repo is input, never
    /// authority: a merge there only proposes an activation.
    BindPolicyRepo {
        repo: String,
        /// The repo's default branch as the org's GitHub reader saw it when this was proposed.
        #[serde(default)]
        default_branch: String,
    },
    /// The org's GitHub reader App (O3, O4): its app and installation ids. Its private key and webhook secret are posted
    /// apart, sealed.
    SetGithubReader {
        app_id: u64,
        installation_id: u64,
    },
}

/// Where an org executor runs (05 §5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorMode {
    /// Hosted by Reins Cloud, one isolated runtime per tenant.
    Cloud,
    /// A single-tenant deployment in the org's own region.
    Dedicated,
    /// In the org's own network.
    SelfHosted,
}

impl ExecutorMode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cloud => "cloud",
            Self::Dedicated => "dedicated",
            Self::SelfHosted => "self_hosted",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        [Self::Cloud, Self::Dedicated, Self::SelfHosted].into_iter().find(|m| m.as_str() == s)
    }
}

impl AdminAction {
    /// What the proposer and the approver must both be able to do. An action's risk depends on whom it touches, so
    /// `target_roles` are the roles the subject holds now (a member's direct and group roles; a group's roles; empty
    /// for actions without a subject): removing a member who holds a privileged role takes [`Capability::AssignAdmins`],
    /// as granting that role would.
    #[must_use]
    pub fn capability(&self, target_roles: &[Role]) -> Capability {
        let touches_privileged = target_roles.iter().any(|r| r.is_privileged());
        match self {
            Self::GrantRole {
                role,
                ..
            }
            | Self::RevokeRole {
                role,
                ..
            } => {
                if role.is_admin() {
                    Capability::AssignAdmins
                } else {
                    Capability::AssignRoles
                }
            }
            Self::SetIdentity {
                ..
            }
            | Self::SetTier {
                ..
            }
            | Self::RemoveDomain {
                ..
            }
            | Self::SetTwoPerson {
                ..
            }
            | Self::BindPolicyRepo {
                ..
            } => Capability::SetUpOrg,
            Self::RemoveMember {
                ..
            }
            | Self::ConfirmDirectoryRemovals {
                ..
            } => {
                if touches_privileged {
                    Capability::AssignAdmins
                } else {
                    Capability::ManageMembers
                }
            }
            Self::ActivatePolicy {
                ..
            }
            | Self::SetRetention {
                ..
            } => Capability::ActivatePolicy,
            Self::RegisterExecutor {
                ..
            }
            | Self::RemoveExecutor {
                ..
            }
            | Self::SetGithubReader {
                ..
            } => Capability::ManageConnections,
        }
    }

    /// The admin actions 06 §3 names: they take two admins' device signatures (the proposer's and the approver's).
    #[must_use]
    pub fn needs_device_signatures(&self) -> bool {
        matches!(
            self,
            Self::ActivatePolicy { .. }
                | Self::SetRetention { .. }
                | Self::RegisterExecutor { .. }
                | Self::RemoveExecutor { .. }
        )
    }

    /// Shapes refused whoever proposes them: an admin role granted to a group (anyone the identity provider adds to
    /// it would become an admin without an admin's approval).
    pub fn check_shape(&self) -> Result<(), &'static str> {
        if let Self::GrantRole {
            subject: Subject::Group(_),
            role,
        } = self
            && role.is_admin()
        {
            return Err("Org owner and Security admin are given to people, not groups");
        }
        Ok(())
    }
}

/// The longest domain name (RFC 1035).
const MAX_DOMAIN: usize = 253;

/// A domain as Reins keeps it: lowercase ASCII (IDNA), no trailing dot, at least two labels of letters, digits and
/// inner hyphens. `None` for anything else, a URL or an address included.
#[must_use]
pub fn normalize_domain(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let raw = raw.strip_suffix('.').unwrap_or(raw);
    if raw.is_empty() || raw.contains(['/', ':', '@', ' ', '\\', '?', '#', '%']) {
        return None;
    }
    let ascii = idna::domain_to_ascii(raw).ok()?;
    if ascii.len() > MAX_DOMAIN {
        return None;
    }
    let labels: Vec<&str> = ascii.split('.').collect();
    let label_ok = |l: &&str| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    };
    // The last label is a TLD: never all digits (that would be an IP address).
    let tld_ok = labels.last().is_some_and(|t| !t.bytes().all(|b| b.is_ascii_digit()));
    (labels.len() >= 2 && labels.iter().all(label_ok) && tld_ok).then_some(ascii)
}

/// The normalized domain of an email address.
#[must_use]
pub fn email_domain(email: &str) -> Option<String> {
    let (local, domain) = email.trim().rsplit_once('@')?;
    if local.is_empty() {
        return None;
    }
    normalize_domain(domain)
}

/// The kinds of attestation a device can bring. O1 records them; verifying them is O2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttestationKind {
    /// iOS `DCAppAttestService.attestKey` over SHA-256 of the challenge (CBOR, base64).
    AppAttest,
    /// Android Keystore key attestation: the certificate chain of the enrolled key (DER certificates, base64, joined
    /// with `,`), with the challenge as the attestation challenge.
    AndroidKeyAttestation,
    /// A Play Integrity token whose nonce is the challenge.
    PlayIntegrity,
    /// The platform offered none (simulator, unsupported device, no entitlement).
    None,
}

/// The largest attestation blob kept.
pub const MAX_ATTESTATION_CHARS: usize = 64 * 1024;
/// Attestations per enrollment.
pub const MAX_ATTESTATIONS: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attestation {
    pub kind: AttestationKind,
    #[serde(default)]
    pub data: String,
}

/// What a device says about itself (06 §6). Recorded at enrollment and when it changes; decisions use it from O2.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Posture {
    /// A passcode, PIN, pattern or biometric lock is set.
    pub screen_lock: bool,
    /// `iOS`, `Android`.
    pub os_name: String,
    pub os_version: String,
    /// Root or jailbreak signs; `None` when the check could not run.
    #[serde(default)]
    pub rooted: Option<bool>,
    pub app_version: String,
    /// Unix seconds.
    pub collected_at: i64,
}

impl Posture {
    /// Short fields only: nothing here may carry more than a version string.
    pub fn validate(&self) -> Result<(), &'static str> {
        let short = |s: &str| !s.is_empty() && s.len() <= 64 && !s.chars().any(char::is_control);
        if !short(&self.os_name) || !short(&self.os_version) || !short(&self.app_version) {
            return Err("The posture's names and versions must be 1 to 64 characters");
        }
        Ok(())
    }
}

/// `POST /reins/api/org/enroll/challenge`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollmentChallenge {
    /// base64url, single use.
    pub challenge: String,
    pub expires_at: i64,
}

/// `POST /reins/api/org/devices`: the device key made in secure hardware for this org, and what vouches for it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollmentRequest {
    pub challenge: String,
    /// The public key, SubjectPublicKeyInfo DER in base64.
    pub device_key: String,
    /// `p256`.
    pub key_algorithm: String,
    /// The private key lives in the Secure Enclave, StrongBox or a TEE.
    pub hardware: bool,
    #[serde(default)]
    pub attestations: Vec<Attestation>,
    pub posture: Posture,
    /// The device key's signature over [`enrollment_message`] of the challenge (ECDSA P-256 with SHA-256, DER, base64):
    /// proof that the phone holds the key it enrolls.
    pub signature: String,
}

/// What the device key signs to enroll: a fixed context and the challenge.
#[must_use]
pub fn enrollment_message(challenge: &str) -> Vec<u8> {
    let mut m = b"reins/org-enroll/v1\n".to_vec();
    m.extend_from_slice(challenge.as_bytes());
    m
}

/// What a platform attestation of an enrollment vouches for (App Attest's client data hash; Android's key attestation
/// challenge): SHA-256 of [`enrollment_message`] of the challenge followed by the enrolled key (SPKI DER). So an
/// attestation made for one key can't be replayed for another, and O2's verifier checks it against the recorded key.
#[must_use]
pub fn attestation_binding(challenge: &str, device_key_der: &[u8]) -> [u8; 32] {
    let mut m = enrollment_message(challenge);
    m.extend_from_slice(device_key_der);
    crate::canon::sha256(&m)
}

/// The only key algorithm enrolled in O1.
pub const KEY_P256: &str = "p256";

impl EnrollmentRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.challenge.is_empty() || self.challenge.len() > 128 {
            return Err("The enrollment has no valid challenge");
        }
        if self.key_algorithm != KEY_P256 {
            return Err("Only P-256 device keys are enrolled");
        }
        if self.device_key.is_empty() || self.device_key.len() > 512 {
            return Err("The device key is missing or too long");
        }
        if self.signature.is_empty() || self.signature.len() > 256 {
            return Err("The enrollment is not signed by the device key");
        }
        if self.attestations.len() > MAX_ATTESTATIONS
            || self.attestations.iter().any(|a| a.data.len() > MAX_ATTESTATION_CHARS)
        {
            return Err("Too many or too large attestations");
        }
        self.posture.validate()
    }
}

/// Which remotes are the org's (08 R6): its GitHub organizations, and whole git hosts it runs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgRoutes {
    #[serde(default)]
    pub github_orgs: Vec<String>,
    #[serde(default)]
    pub hosts: Vec<String>,
}

/// `GET /reins/api/org`: the Managed account's own view of its organization. Answered only to the org's members.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgMe {
    pub org_id: String,
    pub name: String,
    pub tier: Tier,
    pub identity: IdentityKind,
    pub domains: Vec<String>,
    pub roles: Vec<Role>,
    /// This device is enrolled (and not revoked).
    pub enrolled: bool,
    #[serde(default)]
    pub routes: OrgRoutes,
}

/// Which profile a request belongs to (08 R6).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ProfileRef {
    Personal,
    /// The org's id, as the bindings name it.
    Managed(String),
}

/// A git remote reduced to what routing compares: the host (lowercase, no user, port or trailing dot; every
/// GitHub-owned host as `github.com`) and the repository path (no leading or trailing slashes, no `.git`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remote {
    pub host: String,
    pub path: String,
}

/// Whether `host` (lowercase) is GitHub's own: `github.com` and any of its subdomains (`www`, `ssh`, `api`). They all
/// name the same owners, so an org's GitHub organization is matched on every one of them.
fn github_owned(host: &str) -> bool {
    host == "github.com" || host.ends_with(".github.com")
}

/// The host part of an authority (`user@host:port`, `[v6]:port`), lowercase, without the trailing dot.
fn bare_host(authority: &str) -> String {
    let after_user = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if let Some(rest) = after_user.strip_prefix('[') {
        rest.split_once(']').map_or(rest, |(h, _)| h)
    } else {
        after_user.split_once(':').map_or(after_user, |(h, _)| h)
    };
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if github_owned(&host) {
        "github.com".to_owned()
    } else {
        host
    }
}

fn clean_path(path: &str) -> String {
    let path = path.split(['?', '#']).next().unwrap_or_default().trim().trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path).trim_end_matches('/');
    path.to_owned()
}

impl Remote {
    /// Parses the remote forms git accepts: `https://[user@]host[:port]/path`, `ssh://[user@]host[:port]/path`,
    /// `git://…`, and the scp form `[user@]host:path`. `None` for a local path or anything else.
    #[must_use]
    pub fn parse(url: &str) -> Option<Self> {
        let url = url.trim();
        let (authority, path) = if let Some((scheme, rest)) = url.split_once("://") {
            if !matches!(scheme.to_ascii_lowercase().as_str(), "https" | "http" | "ssh" | "git" | "git+ssh") {
                return None;
            }
            rest.split_once('/').unwrap_or((rest, ""))
        } else {
            // scp form: a colon before any slash, and not a Windows drive letter.
            let colon = url.find(':')?;
            if url[..colon].contains('/') || colon < 2 {
                return None;
            }
            (&url[..colon], &url[colon + 1..])
        };
        let host = bare_host(authority);
        if host.is_empty() {
            return None;
        }
        Some(Self {
            host,
            path: clean_path(path),
        })
    }

    /// From a host and path already split (the desktop proxy's routes), normalized the same way.
    #[must_use]
    pub fn from_parts(host: &str, path: &str) -> Self {
        Self {
            host: bare_host(host),
            path: clean_path(path),
        }
    }
}

/// The profile `remote` belongs to: an org's when its host is one of the org's hosts, or it is on GitHub and its owner
/// is one of the org's GitHub organizations (case-insensitive, on every GitHub-owned host); else Personal.
#[must_use]
pub fn route_remote(bindings: &[(String, OrgRoutes)], remote: &Remote) -> ProfileRef {
    let owner = remote.path.split('/').next().unwrap_or_default();
    for (org, routes) in bindings {
        if routes.hosts.iter().any(|h| bare_host(h) == remote.host) {
            return ProfileRef::Managed(org.clone());
        }
        if remote.host == "github.com"
            && !owner.is_empty()
            && routes.github_orgs.iter().any(|o| o.trim().eq_ignore_ascii_case(owner))
        {
            return ProfileRef::Managed(org.clone());
        }
    }
    ProfileRef::Personal
}

/// The `prev_hash` of an org's first event.
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// One admin event as it is chained: the fields in this order, then the previous event's hash.
#[derive(Clone, Copy, Debug)]
pub struct EventFields<'a> {
    pub org: &'a str,
    pub seq: i64,
    pub at: i64,
    pub kind: &'a str,
    pub actor: Option<&'a str>,
    pub subject: Option<&'a str>,
    /// The stored detail text (JSON), byte for byte.
    pub detail: &'a str,
}

/// The hash of an admin event (lowercase hex SHA-256 of its canonical encoding and `prev_hash`). O2's audit log and
/// `reins-audit verify` recompute it to find an edited, removed or reordered event.
#[must_use]
pub fn event_hash(event: &EventFields<'_>, prev_hash: &str) -> String {
    let mut c = crate::canon::Canon::new(crate::canon::contexts::ORG_EVENT);
    c.str(event.org).i64(event.seq).i64(event.at).str(event.kind);
    for v in [event.actor, event.subject] {
        match v {
            Some(v) => c.bool(true).str(v),
            None => c.bool(false),
        };
    }
    c.str(event.detail).str(prev_hash);
    crate::canon::hex32(&crate::canon::sha256(&c.finish()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_allow_their_identity_kinds() {
        assert!(Tier::Team.allows(IdentityKind::GoogleWorkspace));
        assert!(Tier::Team.allows(IdentityKind::GithubOrg));
        assert!(!Tier::Team.allows(IdentityKind::Sso));
        for tier in [Tier::Business, Tier::Enterprise] {
            for id in IdentityKind::ALL {
                assert!(tier.allows(id), "{tier:?} {id:?}");
            }
            assert!(tier.scim());
        }
        assert!(!Tier::Team.scim());
        for t in Tier::ALL {
            assert_eq!(Tier::parse(t.as_str()), Some(t));
        }
        assert_eq!(serde_json::to_string(&IdentityKind::GithubOrg).unwrap(), "\"github_org\"");
    }

    #[test]
    fn owner_assigns_admins_and_only_owners_do() {
        assert!(role_grants(Role::OrgOwner, Capability::AssignAdmins));
        assert!(!role_grants(Role::SecurityAdmin, Capability::AssignAdmins));
        assert!(role_grants(Role::SecurityAdmin, Capability::AssignRoles));
        assert!(!role_grants(Role::Auditor, Capability::ManageMembers));
        assert!(role_grants(Role::Auditor, Capability::ViewAdminEvents));
        assert!(!role_grants(Role::PolicyAuthor, Capability::ActivatePolicy));
        assert!(!role_grants(Role::ConnectionAdmin, Capability::AuthorPolicy));
        assert!(role_grants(Role::Approver, Capability::Approve));
        for cap in [Capability::SetUpOrg, Capability::Approve, Capability::ViewMembers, Capability::ManageMembers] {
            assert!(!role_grants(Role::Member, cap));
        }
        for r in Role::ALL {
            assert_eq!(Role::parse(r.as_str()), Some(r));
        }
    }

    #[test]
    fn o2_admin_actions_need_security_or_connections_and_device_signatures() {
        let activate = AdminAction::ActivatePolicy {
            revision: 3,
            hash: "h".to_owned(),
            report: None,
        };
        // O2's stored changes (no report) still read.
        let old: AdminAction = serde_json::from_str(r#"{"kind":"activate_policy","revision":3,"hash":"h"}"#).unwrap();
        assert_eq!(old, activate);
        assert_eq!(serde_json::to_string(&activate).unwrap(), r#"{"kind":"activate_policy","revision":3,"hash":"h"}"#);
        let bind = AdminAction::BindPolicyRepo {
            repo: "acme/reins-policy".to_owned(),
            default_branch: "main".to_owned(),
        };
        assert_eq!(bind.capability(&[]), Capability::SetUpOrg);
        let reader = AdminAction::SetGithubReader {
            app_id: 1,
            installation_id: 2,
        };
        assert_eq!(reader.capability(&[]), Capability::ManageConnections);
        assert_eq!(activate.capability(&[]), Capability::ActivatePolicy);
        assert!(activate.needs_device_signatures());
        let exec = AdminAction::RegisterExecutor {
            executor_id: "e1".to_owned(),
            mode: ExecutorMode::Cloud,
            public_key: "k".to_owned(),
        };
        assert_eq!(exec.capability(&[]), Capability::ManageConnections);
        assert!(exec.needs_device_signatures());
        assert!(
            !AdminAction::SetTwoPerson {
                on: true
            }
            .needs_device_signatures()
        );
        let json = serde_json::to_value(&exec).unwrap();
        assert_eq!(json["kind"], "register_executor");
        assert_eq!(json["mode"], "cloud");
        assert_eq!(ExecutorMode::parse("self_hosted"), Some(ExecutorMode::SelfHosted));
    }

    #[test]
    fn admin_actions_name_the_capability_they_need() {
        let grant = |role| AdminAction::GrantRole {
            subject: Subject::Member("m1".to_owned()),
            role,
        };
        assert_eq!(grant(Role::SecurityAdmin).capability(&[]), Capability::AssignAdmins);
        assert_eq!(grant(Role::OrgOwner).capability(&[]), Capability::AssignAdmins);
        assert_eq!(grant(Role::Auditor).capability(&[]), Capability::AssignRoles);
        assert_eq!(
            AdminAction::SetTier {
                tier: Tier::Business
            }
            .capability(&[]),
            Capability::SetUpOrg
        );
        let json = serde_json::to_value(grant(Role::Approver)).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"kind": "grant_role", "subject": {"type": "member", "id": "m1"},
                                            "role": "approver"})
        );
        assert_eq!(serde_json::from_value::<AdminAction>(json).unwrap(), grant(Role::Approver));
    }

    #[test]
    fn removing_a_privileged_member_takes_what_making_one_takes() {
        let remove = AdminAction::RemoveMember {
            member: "m".to_owned(),
        };
        assert_eq!(remove.capability(&[]), Capability::ManageMembers);
        assert_eq!(remove.capability(&[Role::Member, Role::Approver, Role::Auditor]), Capability::ManageMembers);
        for privileged in [Role::OrgOwner, Role::SecurityAdmin, Role::ConnectionAdmin, Role::PolicyAuthor] {
            assert_eq!(remove.capability(&[Role::Member, privileged]), Capability::AssignAdmins, "{privileged:?}");
        }
        // A Security admin manages members but cannot remove another Security admin.
        assert!(role_grants(Role::SecurityAdmin, Capability::ManageMembers));
        assert!(!role_grants(Role::SecurityAdmin, remove.capability(&[Role::SecurityAdmin])));
        assert!(role_grants(Role::OrgOwner, remove.capability(&[Role::SecurityAdmin])));
    }

    #[test]
    fn admin_roles_go_to_people_not_groups() {
        let to_group = |role| AdminAction::GrantRole {
            subject: Subject::Group("g".to_owned()),
            role,
        };
        assert!(to_group(Role::OrgOwner).check_shape().is_err());
        assert!(to_group(Role::SecurityAdmin).check_shape().is_err());
        assert!(to_group(Role::Approver).check_shape().is_ok());
        assert!(
            AdminAction::RevokeRole {
                subject: Subject::Group("g".to_owned()),
                role: Role::OrgOwner
            }
            .check_shape()
            .is_ok()
        );
    }

    #[test]
    fn domains_are_normalized_and_bad_ones_refused() {
        assert_eq!(normalize_domain(" Acme.COM. ").as_deref(), Some("acme.com"));
        assert_eq!(normalize_domain("eng.acme.co.uk").as_deref(), Some("eng.acme.co.uk"));
        assert_eq!(normalize_domain("münchen.de").as_deref(), Some("xn--mnchen-3ya.de"));
        for bad in [
            "com",
            "a..b",
            "-a.com",
            "a-.com",
            "a b.com",
            "acme.com/x",
            "dana@acme.com",
            "",
            ".",
            "1.2.3.4",
            "acme.com:443",
            "*.acme.com",
            "a_b.com",
        ] {
            assert_eq!(normalize_domain(bad), None, "{bad}");
        }
        assert_eq!(normalize_domain(&format!("{}.com", "a".repeat(64))), None);
    }

    #[test]
    fn email_domains() {
        assert_eq!(email_domain("Dana@Acme.com").as_deref(), Some("acme.com"));
        assert_eq!(email_domain("a@b@acme.com").as_deref(), Some("acme.com"));
        assert_eq!(email_domain("@acme.com"), None);
        assert_eq!(email_domain("acme.com"), None);
    }

    #[test]
    fn org_remotes_route_to_their_profile_and_others_stay_personal() {
        let bindings = vec![(
            "o1".to_owned(),
            OrgRoutes {
                github_orgs: vec!["acme".to_owned()],
                hosts: vec!["gitlab.acme.com".to_owned()],
            },
        )];
        let managed = ProfileRef::Managed("o1".to_owned());
        let route = |url: &str| route_remote(&bindings, &Remote::parse(url).expect(url));
        for url in [
            "https://github.com/Acme/api",
            "https://github.com/acme/api.git",
            "https://github.com/acme/api/",
            "https://dana:token@github.com/acme/api.git",
            "https://GITHUB.COM./ACME/api",
            "https://github.com:443/acme/api",
            "https://www.github.com/acme/api",
            "git@github.com:acme/api.git",
            "git@GitHub.com:/acme/api",
            "ssh://git@github.com/acme/api.git",
            "ssh://git@ssh.github.com:443/acme/api.git",
            "ssh.github.com:acme/api",
            "git://github.com/acme/api",
            "https://gitlab.acme.com/team/sub/repo.git",
            "git@gitlab.acme.com:team/repo",
            "ssh://git@GitLab.Acme.com:2222/team/repo",
        ] {
            assert_eq!(route(url), managed, "{url}");
        }
        for url in [
            "https://github.com/dkat/reins",
            "https://github.com.evil.com/acme/api",
            "git@github.com.evil.com:acme/api",
            "https://evilgithub.com/acme/api",
            "https://gitlab.com/acme/x",
            "https://github.com/acmes/x",
            "https://github.com/x/acme",
            "https://gitlab.acme.com.evil.io/team/repo",
        ] {
            assert_eq!(route(url), ProfileRef::Personal, "{url}");
        }
        assert_eq!(route_remote(&[], &Remote::parse("https://github.com/acme/api").unwrap()), ProfileRef::Personal);
        assert_eq!(route_remote(&bindings, &Remote::from_parts("SSH.GitHub.com:443", "/acme/api.git/")), managed);
    }

    #[test]
    fn remotes_parse_or_are_refused() {
        assert_eq!(
            Remote::parse("ssh://git@ssh.github.com:443/acme/api.git"),
            Some(Remote {
                host: "github.com".to_owned(),
                path: "acme/api".to_owned()
            })
        );
        assert_eq!(Remote::parse("https://[::1]:8080/a/b").unwrap().host, "::1");
        for bad in ["/home/dana/repo", "C:/repo", "file:///repo", "../repo", "", "ftp://github.com/acme/api"] {
            assert_eq!(Remote::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn attestations_are_bound_to_the_enrolled_key() {
        let a = attestation_binding("ch", &[1, 2, 3]);
        assert_ne!(a, attestation_binding("ch", &[1, 2, 4]));
        assert_ne!(a, attestation_binding("other", &[1, 2, 3]));
        // The vector the phones' own tests pin.
        assert_eq!(
            crate::canon::hex32(&attestation_binding("challenge-1", &[0x30, 0x59])),
            crate::canon::hex32(&crate::canon::sha256(b"reins/org-enroll/v1\nchallenge-1\x30\x59"))
        );
    }

    #[test]
    fn event_hashes_cover_every_field_and_the_chain() {
        let e = EventFields {
            org: "o1",
            seq: 1,
            at: 10,
            kind: "org.created",
            actor: Some("u1"),
            subject: None,
            detail: "{}",
        };
        let h = event_hash(&e, GENESIS_HASH);
        assert_eq!(h.len(), 64);
        assert_eq!(h, event_hash(&e, GENESIS_HASH));
        assert_ne!(
            h,
            event_hash(
                &EventFields {
                    seq: 2,
                    ..e
                },
                GENESIS_HASH
            )
        );
        assert_ne!(
            h,
            event_hash(
                &EventFields {
                    actor: None,
                    ..e
                },
                GENESIS_HASH
            )
        );
        assert_ne!(
            h,
            event_hash(
                &EventFields {
                    actor: None,
                    subject: Some("u1"),
                    ..e
                },
                GENESIS_HASH
            )
        );
        assert_ne!(
            h,
            event_hash(
                &EventFields {
                    detail: "{ }",
                    ..e
                },
                GENESIS_HASH
            )
        );
        assert_ne!(h, event_hash(&e, &h));
    }

    fn enrollment() -> EnrollmentRequest {
        EnrollmentRequest {
            challenge: "c".repeat(43),
            device_key: "MFkw".to_owned(),
            key_algorithm: KEY_P256.to_owned(),
            hardware: true,
            attestations: vec![Attestation {
                kind: AttestationKind::AppAttest,
                data: "o2Nm".to_owned(),
            }],
            posture: Posture {
                screen_lock: true,
                os_name: "iOS".to_owned(),
                os_version: "26.1".to_owned(),
                rooted: Some(false),
                app_version: "1.4.0".to_owned(),
                collected_at: 1,
            },
            signature: "MEUC".to_owned(),
        }
    }

    #[test]
    fn enrollment_requests_are_bounded() {
        assert_eq!(enrollment().validate(), Ok(()));
        let mut e = enrollment();
        e.challenge.clear();
        assert!(e.validate().is_err());
        let mut e = enrollment();
        e.attestations[0].data = "a".repeat(70 * 1024);
        assert!(e.validate().is_err());
        let mut e = enrollment();
        e.device_key.clear();
        assert!(e.validate().is_err());
        let mut e = enrollment();
        e.key_algorithm = "rsa".to_owned();
        assert!(e.validate().is_err());
        let mut e = enrollment();
        e.signature.clear();
        assert!(e.validate().is_err());
        assert_eq!(enrollment_message("abc"), b"reins/org-enroll/v1\nabc");
        let mut e = enrollment();
        e.posture.os_version = "1\n2".to_owned();
        assert!(e.validate().is_err());
        let json = serde_json::to_value(enrollment()).unwrap();
        assert_eq!(json["attestations"][0]["kind"], "app_attest");
    }
}
