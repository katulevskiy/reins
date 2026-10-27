//! Who may sign in to a Managed account, and how (05 §2: "Members sign in to their Managed profile only through SSO;
//! passwords are disabled for Managed profiles"; 08 R11: Team signs in with Google Workspace or a GitHub org).
//!
//! The rules ([`decide`]), for a sign-in whose email is on a domain an org verified, or by an account that is already
//! an org's member:
//!
//! - an existing account that is **not** the org's member stays a Personal account and is never captured (nothing
//!   about it reaches the org);
//! - otherwise the sign-in must come through the org's identity: SSO through the org's own WorkOS organization, or
//!   the Team provider (Google; GitHub with an active membership of the org's GitHub organization);
//! - a removed (revoked) member is refused;
//! - a new account joins (just in time), unless the org provisions from a directory (SCIM), whose members exist
//!   before they first sign in;
//! - an org that has not set up sign-in yet lets in only its existing members.

use reins_proto::org::{IdentityKind, Tier, email_domain};
use serde_json::json;

use super::{enabled, event};
use crate::{
    api::{ApiResult, EmptyResult},
    db::{
        DbConn,
        models::{
            Membership, OIDCAuthenticatedUser, User, UserId,
            reins_org::{self, ReinsOrg},
        },
    },
};

/// Where `/identity/connect/authorize?domain_hint=` sends a work account's sign-in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrgHint {
    /// The org's WorkOS organization (its SSO connection).
    pub organization_id: Option<String>,
    /// A Team org's provider: `GoogleOAuth` or `GitHubOAuth`.
    pub provider: Option<&'static str>,
}

/// The facts about the org that [`decide`] needs.
#[derive(Clone, Debug)]
pub struct OrgFacts {
    pub org_uuid: String,
    pub name: String,
    pub tier: Tier,
    pub identity: IdentityKind,
    pub workos_org_id: Option<String>,
    pub has_directory: bool,
}

impl OrgFacts {
    fn of(org: &ReinsOrg, name: String) -> Self {
        Self {
            org_uuid: org.org_uuid.clone(),
            name,
            tier: org.tier(),
            identity: org.identity(),
            workos_org_id: org.workos_org_id.clone(),
            has_directory: org.directory_id.is_some(),
        }
    }
}

/// The signing-in account, as the org sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Existing {
    NoAccount,
    NotMember,
    Member {
        active: bool,
    },
}

/// How this sign-in happened.
#[derive(Clone, Copy, Debug, Default)]
pub struct SignInFacts<'a> {
    pub method: Option<&'a str>,
    pub organization_id: Option<&'a str>,
    /// For a GitHub org: whether the user is an active member (`None`: not checked, no token).
    pub github_member: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Personal,
    Managed {
        org: String,
        /// The account joins the org at this sign-in.
        jit: bool,
        /// The sign-in came through the org's own WorkOS organization (its SSO or Team provider), as WorkOS says, not
        /// as the email's domain suggests. Only such a sign-in is exempt from the passkey rule.
        verified: bool,
    },
}

impl Decision {
    /// Whether this sign-in proved itself through the org's verified identity.
    #[must_use]
    pub fn through_org_identity(&self) -> bool {
        matches!(
            self,
            Self::Managed {
                verified: true,
                ..
            }
        )
    }
}

/// The rules in the module docs. `Err` carries the text shown to the person signing in.
pub fn decide(org: Option<&OrgFacts>, existing: Existing, sign_in: &SignInFacts<'_>) -> Result<Decision, String> {
    let Some(org) = org else {
        return Ok(Decision::Personal);
    };
    let active = match existing {
        Existing::NotMember => return Ok(Decision::Personal),
        Existing::Member {
            active: false,
        } => return Err("Your organization removed this account.".to_owned()),
        Existing::Member {
            active: true,
        } => true,
        Existing::NoAccount => false,
    };
    // Not set up: only its members (its creator) get in, as they signed in before.
    if org.identity == IdentityKind::None {
        return if active {
            Ok(Decision::Managed {
                org: org.org_uuid.clone(),
                jit: false,
                verified: false,
            })
        } else {
            Err(format!("{} hasn't set up sign-in yet. Ask your IT team.", org.name))
        };
    }
    // Every identity goes through the tenant's own WorkOS organization (bound only once WorkOS confirmed it holds one of
    // the tenant's verified domains): WorkOS names it on the sign-in. The method then says which way.
    let through_workos_org = org.workos_org_id.is_some() && sign_in.organization_id == org.workos_org_id.as_deref();
    let through_org = through_workos_org
        && match org.identity {
            IdentityKind::None => false,
            IdentityKind::Sso => sign_in.method == Some("SSO"),
            IdentityKind::GoogleWorkspace => sign_in.method == Some("GoogleOAuth"),
            IdentityKind::GithubOrg => sign_in.method == Some("GitHubOAuth") && sign_in.github_member == Some(true),
        };
    if !through_org {
        let how = match org.identity {
            IdentityKind::GoogleWorkspace => "Google",
            IdentityKind::GithubOrg => "GitHub (as a member of its GitHub organization)",
            _ => "its SSO",
        };
        return Err(format!("{} manages this account. Sign in with {how}.", org.name));
    }
    if !active && org.tier.scim() && org.has_directory {
        return Err(format!("{} adds its members from its directory. Ask your IT team to add you.", org.name));
    }
    Ok(Decision::Managed {
        org: org.org_uuid.clone(),
        jit: !active,
        verified: true,
    })
}

/// The org that verified the domain of `email`.
async fn org_of_domain(email: &str, conn: &DbConn) -> Option<String> {
    let domain = email_domain(email)?;
    conn.run(move |c| reins_org::q_verified_owner(c, &domain)).await.ok().flatten()
}

/// Where a work account's sign-in goes, for `/identity/connect/authorize?domain_hint=`: `None` for a domain no org
/// verified (or one whose sign-in isn't set up), and the default sign-in happens.
pub async fn hint_for(domain: &str, conn: &DbConn) -> Option<OrgHint> {
    if !enabled() {
        return None;
    }
    let domain = reins_proto::org::normalize_domain(domain)?;
    let org = conn
        .run(move |c| {
            let owner = reins_org::q_verified_owner(c, &domain)?;
            owner.map(|o| reins_org::q_org(c, &o)).transpose().map(Option::flatten)
        })
        .await
        .ok()
        .flatten()?;
    let provider = match org.identity() {
        IdentityKind::Sso => None,
        IdentityKind::GoogleWorkspace => Some("GoogleOAuth"),
        IdentityKind::GithubOrg => Some("GitHubOAuth"),
        IdentityKind::None => return None,
    };
    org.workos_org_id.map(|id| OrgHint {
        organization_id: Some(id),
        provider,
    })
}

/// Applies [`decide`] to a sign-in: the org is the one the existing account is a member of, else the one that
/// verified the email's domain.
pub async fn check(user: &OIDCAuthenticatedUser, existing: Option<&User>, conn: &DbConn) -> ApiResult<Decision> {
    if !enabled() {
        return Ok(Decision::Personal);
    }
    let existing_uuid = existing.map(|u| u.uuid.to_string());
    let domain_org = org_of_domain(&user.email, conn).await;
    let loaded = conn
        .run(move |c| -> Result<Option<(OrgFacts, Existing)>, diesel::result::Error> {
            let member_of = match &existing_uuid {
                Some(u) => reins_org::q_orgs_of_user_any(c, u)?,
                None => Vec::new(),
            };
            let org_uuid = member_of.first().cloned().or(domain_org);
            let Some(org_uuid) = org_uuid else {
                return Ok(None);
            };
            let Some(org) = reins_org::q_org(c, &org_uuid)? else {
                return Ok(None);
            };
            let name = reins_org::q_org_name(c, &org_uuid)?.unwrap_or_default();
            let existing = match &existing_uuid {
                None => Existing::NoAccount,
                Some(u) => match reins_org::q_member_state(c, &org_uuid, u)? {
                    None => Existing::NotMember,
                    Some(m) => Existing::Member {
                        active: m.is_active(),
                    },
                },
            };
            Ok(Some((OrgFacts::of(&org, name), existing)))
        })
        .await;
    let loaded = match loaded {
        Ok(l) => l,
        Err(e) => err!(format!("Could not check the organization: {e}")),
    };
    let Some((org, existing)) = loaded else {
        return Ok(Decision::Personal);
    };
    let workos = user.workos.clone().unwrap_or_default();
    let mut github_member = None;
    if org.identity == IdentityKind::GithubOrg
        && workos.method.as_deref() == Some("GitHubOAuth")
        && let (Some(token), Some(gh_org)) =
            (workos.provider_token.as_deref(), github_org_of(&org.org_uuid, conn).await)
    {
        github_member = match super::github::checker().is_member(token, &gh_org).await {
            Ok(m) => Some(m),
            Err(e) => {
                warn!("GitHub membership check for org {}: {e}", org.org_uuid);
                None
            }
        };
    }
    let facts = SignInFacts {
        method: workos.method.as_deref(),
        organization_id: workos.organization_id.as_deref(),
        github_member,
    };
    match decide(Some(&org), existing, &facts) {
        Ok(d) => Ok(d),
        Err(message) => {
            info!("Org {}: refused a sign-in ({:?}, {:?})", org.org_uuid, facts.method, existing);
            err!(message)
        }
    }
}

async fn github_org_of(org: &str, conn: &DbConn) -> Option<String> {
    let org = org.to_owned();
    conn.run(move |c| reins_org::q_org(c, &org)).await.ok().flatten().and_then(|o| o.github_org)
}

/// After the account is known: a just-in-time member joins the org.
pub async fn after_sign_in(user: &User, decision: &Decision, conn: &DbConn) -> EmptyResult {
    let Decision::Managed {
        org,
        jit: true,
        ..
    } = decision
    else {
        return Ok(());
    };
    let (org, user_uuid) = (org.clone(), user.uuid.to_string());
    let now = super::super::now_unix();
    let done = conn
        .run(move |c| {
            use diesel::Connection;
            c.transaction(|c| -> Result<(), super::OrgError> {
                if !reins_org::q_orgs_of_user_any(c, &user_uuid)?.is_empty() {
                    return Ok(());
                }
                let m = Membership::new(user_uuid.clone().into(), org.clone().into(), None);
                reins_org::q_add_member(c, &m, Some(&user_uuid), now)?;
                event(c, &org, "member.joined", Some(&user_uuid), Some(m.uuid.as_ref()), &json!({"jit": true}), now)
            })
        })
        .await;
    if let Err(e) = done {
        err!(format!("Could not add you to your organization: {e:?}"));
    }
    Ok(())
}

/// Whether `user` is a work account: a member of an org, in whatever state (removed members included: a work
/// account never signs in with a password). Fails closed on a database error.
pub async fn is_managed_member(user: &UserId, conn: &DbConn) -> bool {
    if !enabled() {
        return false;
    }
    let user = user.to_string();
    !matches!(conn.run(move |c| reins_org::q_orgs_of_user_any(c, &user)).await, Ok(orgs) if orgs.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn org(identity: IdentityKind, tier: Tier) -> OrgFacts {
        OrgFacts {
            org_uuid: "o1".to_owned(),
            name: "Acme".to_owned(),
            tier,
            identity,
            workos_org_id: Some("org_ACME".to_owned()),
            has_directory: false,
        }
    }

    fn via(method: &str, org: Option<&'static str>) -> SignInFacts<'static> {
        SignInFacts {
            method: Some(Box::leak(method.to_owned().into_boxed_str())),
            organization_id: org,
            github_member: None,
        }
    }

    const MEMBER: Existing = Existing::Member {
        active: true,
    };

    fn managed(jit: bool) -> Result<Decision, String> {
        Ok(Decision::Managed {
            org: "o1".to_owned(),
            jit,
            verified: true,
        })
    }

    #[test]
    fn sso_orgs_require_their_own_workos_org() {
        let o = org(IdentityKind::Sso, Tier::Business);
        assert_eq!(decide(Some(&o), MEMBER, &via("SSO", Some("org_ACME"))), managed(false));
        assert_eq!(decide(Some(&o), Existing::NoAccount, &via("SSO", Some("org_ACME"))), managed(true));
        for bad in [
            via("SSO", Some("org_EVIL")),
            via("SSO", None),
            via("Passkey", None),
            via("GoogleOAuth", None),
            via("Password", Some("org_ACME")),
        ] {
            let err = decide(Some(&o), MEMBER, &bad).unwrap_err();
            assert!(err.contains("Acme manages this account"), "{err}");
        }
        let mut unbound = o.clone();
        unbound.workos_org_id = None;
        assert!(decide(Some(&unbound), MEMBER, &via("SSO", None)).is_err());
    }

    #[test]
    fn team_google_requires_google() {
        let o = org(IdentityKind::GoogleWorkspace, Tier::Team);
        assert_eq!(decide(Some(&o), Existing::NoAccount, &via("GoogleOAuth", Some("org_ACME"))), managed(true));
        // A Google account with a work address that WorkOS doesn't count in the org (a consumer account) is refused.
        assert!(decide(Some(&o), Existing::NoAccount, &via("GoogleOAuth", None)).is_err());
        assert!(decide(Some(&o), MEMBER, &via("GitHubOAuth", Some("org_ACME"))).is_err());
        assert!(decide(Some(&o), MEMBER, &via("MagicAuth", None)).is_err());
    }

    #[test]
    fn team_github_requires_active_membership() {
        let o = org(IdentityKind::GithubOrg, Tier::Team);
        let mut s = via("GitHubOAuth", Some("org_ACME"));
        assert!(decide(Some(&o), MEMBER, &s).is_err(), "unchecked");
        s.github_member = Some(false);
        assert!(decide(Some(&o), MEMBER, &s).is_err());
        s.github_member = Some(true);
        assert_eq!(decide(Some(&o), MEMBER, &s), managed(false));
        let mut other_org = via("GitHubOAuth", Some("org_EVIL"));
        other_org.github_member = Some(true);
        assert!(decide(Some(&o), MEMBER, &other_org).is_err());
        let mut google = via("GoogleOAuth", Some("org_ACME"));
        google.github_member = Some(true);
        assert!(decide(Some(&o), MEMBER, &google).is_err());
    }

    #[test]
    fn an_existing_personal_account_is_not_captured() {
        for identity in IdentityKind::ALL {
            let o = org(identity, Tier::Business);
            // Whatever way it signs in, an account that isn't the org's member stays Personal.
            for s in [via("Passkey", None), via("SSO", Some("org_ACME")), via("GoogleOAuth", None)] {
                assert_eq!(decide(Some(&o), Existing::NotMember, &s), Ok(Decision::Personal));
            }
        }
    }

    #[test]
    fn suspended_members_are_refused() {
        let o = org(IdentityKind::Sso, Tier::Business);
        let err = decide(
            Some(&o),
            Existing::Member {
                active: false,
            },
            &via("SSO", Some("org_ACME")),
        )
        .unwrap_err();
        assert!(err.contains("removed"), "{err}");
    }

    #[test]
    fn business_with_a_directory_needs_provisioning_first() {
        let mut o = org(IdentityKind::Sso, Tier::Business);
        o.has_directory = true;
        assert!(decide(Some(&o), Existing::NoAccount, &via("SSO", Some("org_ACME"))).unwrap_err().contains("IT team"));
        assert_eq!(decide(Some(&o), MEMBER, &via("SSO", Some("org_ACME"))), managed(false));
        // A Team org has no directory, whatever the row says.
        let mut team = org(IdentityKind::GoogleWorkspace, Tier::Team);
        team.has_directory = true;
        assert_eq!(decide(Some(&team), Existing::NoAccount, &via("GoogleOAuth", Some("org_ACME"))), managed(true));
    }

    #[test]
    fn an_org_without_sign_in_lets_in_only_its_members() {
        let o = org(IdentityKind::None, Tier::Business);
        // Let in, but not through a verified identity: the passkey rule still applies to this sign-in.
        let d = decide(Some(&o), MEMBER, &via("Passkey", None)).unwrap();
        assert!(matches!(
            d,
            Decision::Managed {
                verified: false,
                ..
            }
        ));
        assert!(!d.through_org_identity());
        assert!(decide(Some(&o), Existing::NoAccount, &via("Passkey", None)).unwrap_err().contains("hasn't set up"));
    }

    #[test]
    fn only_a_sign_in_through_the_org_counts_as_verified() {
        let o = org(IdentityKind::Sso, Tier::Business);
        assert!(decide(Some(&o), MEMBER, &via("SSO", Some("org_ACME"))).unwrap().through_org_identity());
        // A managed-domain address signing in some other way is either refused (a member) or Personal (not a
        // member): never verified.
        assert!(decide(Some(&o), MEMBER, &via("MagicAuth", None)).is_err());
        assert!(!decide(Some(&o), Existing::NotMember, &via("MagicAuth", None)).unwrap().through_org_identity());
        assert!(
            !decide(None, Existing::NoAccount, &via("GoogleOAuth", Some("org_ACME"))).unwrap().through_org_identity()
        );
    }

    fn signed_in(method: &str, org: Option<&str>) -> OIDCAuthenticatedUser {
        OIDCAuthenticatedUser {
            refresh_token: None,
            access_token: String::new(),
            expires_in: None,
            identifier: crate::sso::OIDCIdentifier::from("https://api.workos.com/user_management/c/u".to_owned()),
            email: "dana@acme.com".to_owned(),
            email_verified: Some(true),
            user_name: None,
            session_id: Some("s".to_owned()),
            workos: Some(crate::db::models::WorkosSignIn {
                method: Some(method.to_owned()),
                organization_id: org.map(str::to_owned),
                provider_token: None,
            }),
        }
    }

    /// The passkey rule is waived for the org's own verified sign-in only, never for an address on its domain.
    #[test]
    fn a_managed_domain_address_through_another_idp_still_needs_a_passkey() {
        let o = org(IdentityKind::Sso, Tier::Business);
        for (existing, method, via_org) in
            [(Existing::NotMember, "GoogleOAuth", None), (Existing::NotMember, "MagicAuth", Some("org_EVIL"))]
        {
            let d = decide(Some(&o), existing, &via(method, via_org)).unwrap();
            let user = signed_in(method, via_org);
            assert!(crate::sso_workos::check_passkey(&user, true, d.through_org_identity()).is_err(), "{method}");
        }
        let d = decide(Some(&o), MEMBER, &via("SSO", Some("org_ACME"))).unwrap();
        assert!(
            crate::sso_workos::check_passkey(&signed_in("SSO", Some("org_ACME")), true, d.through_org_identity())
                .is_ok()
        );
    }

    #[test]
    fn unverified_domains_change_nothing() {
        assert_eq!(decide(None, Existing::NoAccount, &via("Password", None)), Ok(Decision::Personal));
        assert_eq!(decide(None, MEMBER, &via("Password", None)), Ok(Decision::Personal));
    }
}
