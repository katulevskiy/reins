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

    /// The roles whose grant and removal only an Org owner may propose or approve.
    #[must_use]
    pub fn is_admin(self) -> bool {
        matches!(self, Self::OrgOwner | Self::SecurityAdmin)
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
}

impl AdminAction {
    /// What the proposer and the approver must both be able to do.
    #[must_use]
    pub fn capability(&self) -> Capability {
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
            } => Capability::SetUpOrg,
            Self::RemoveMember {
                ..
            } => Capability::ManageMembers,
        }
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

/// The profile a git remote at `host`, repository `path` (`owner/name`) belongs to: an org's when the host is one of
/// its hosts, or the host is `github.com` and the owner is one of its GitHub organizations; else Personal.
#[must_use]
pub fn route_remote(bindings: &[(String, OrgRoutes)], host: &str, path: &str) -> ProfileRef {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    let owner = path.trim_start_matches('/').split('/').next().unwrap_or_default();
    for (org, routes) in bindings {
        if routes.hosts.iter().any(|h| h.eq_ignore_ascii_case(&host)) {
            return ProfileRef::Managed(org.clone());
        }
        if host == "github.com" && !owner.is_empty() && routes.github_orgs.iter().any(|o| o.eq_ignore_ascii_case(owner))
        {
            return ProfileRef::Managed(org.clone());
        }
    }
    ProfileRef::Personal
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
    fn admin_actions_name_the_capability_they_need() {
        let grant = |role| AdminAction::GrantRole {
            subject: Subject::Member("m1".to_owned()),
            role,
        };
        assert_eq!(grant(Role::SecurityAdmin).capability(), Capability::AssignAdmins);
        assert_eq!(grant(Role::OrgOwner).capability(), Capability::AssignAdmins);
        assert_eq!(grant(Role::Auditor).capability(), Capability::AssignRoles);
        assert_eq!(
            AdminAction::SetTier {
                tier: Tier::Business
            }
            .capability(),
            Capability::SetUpOrg
        );
        assert_eq!(
            AdminAction::RemoveMember {
                member: "m".to_owned()
            }
            .capability(),
            Capability::ManageMembers
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
        assert_eq!(route_remote(&bindings, "github.com", "Acme/api"), managed);
        assert_eq!(route_remote(&bindings, "GitHub.com.", "/acme/api"), managed);
        assert_eq!(route_remote(&bindings, "gitlab.acme.com", "team/sub/repo"), managed);
        assert_eq!(route_remote(&bindings, "github.com", "dkat/reins"), ProfileRef::Personal);
        assert_eq!(route_remote(&bindings, "github.com.evil.io", "acme/x"), ProfileRef::Personal);
        assert_eq!(route_remote(&bindings, "gitlab.com", "acme/x"), ProfileRef::Personal);
        assert_eq!(route_remote(&bindings, "github.com", "acmes/x"), ProfileRef::Personal);
        assert_eq!(route_remote(&[], "github.com", "acme/api"), ProfileRef::Personal);
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
        e.posture.os_version = "1\n2".to_owned();
        assert!(e.validate().is_err());
        let json = serde_json::to_value(enrollment()).unwrap();
        assert_eq!(json["attestations"][0]["kind"], "app_attest");
    }
}
