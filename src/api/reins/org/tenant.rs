//! Creating an org tenant and verifying its domains (05 §2: "created by an Org owner, who verifies the domain (DNS
//! TXT record)").
//!
//! - The creator's own email domain is the org's first domain, so the account that creates an org is the work account
//!   on that domain (never someone's Personal account on another domain). It starts pending.
//! - A domain counts once a TXT record `reins-domain-verification=<token>` is published at `_reins-challenge.<domain>`:
//!   exactly that record, at exactly that name. A domain covers its own addresses only (`eng.acme.com` is a domain of
//!   its own), public suffixes are refused, and one org at most verifies a domain.

use diesel::prelude::*;
use reins_proto::org::{Capability, IdentityKind, Role, Tier, email_domain, normalize_domain};
use serde_json::json;

use super::{OrgError, dns::TxtResolver, event, random_token, require};
use crate::db::{
    DbConn, DbConnInner,
    models::{
        Membership, Organization,
        reins_org::{self, ReinsOrg, ReinsOrgDomain, ReinsOrgRole, SUBJECT_MEMBER},
    },
    schema::organizations,
};

/// Where the verification record is published, under the domain.
pub const CHALLENGE_LABEL: &str = "_reins-challenge";
/// The record's value, before the token.
pub const RECORD_PREFIX: &str = "reins-domain-verification=";
/// Domains per org.
pub const MAX_DOMAINS: usize = 20;

/// The name the record goes at, for `domain`.
#[must_use]
pub fn record_name(domain: &str) -> String {
    format!("{CHALLENGE_LABEL}.{domain}")
}

/// The record's value, for `token`.
#[must_use]
pub fn record_value(token: &str) -> String {
    format!("{RECORD_PREFIX}{token}")
}

/// A domain an org may claim: well formed and below a public suffix (`acme.co.uk`, not `co.uk`).
pub fn claimable(raw: &str) -> Result<String, OrgError> {
    let domain = normalize_domain(raw).ok_or(OrgError::Invalid("That isn't a domain name."))?;
    // `psl::domain` is the registrable part (`acme.co.uk` of `eng.acme.co.uk`); a public suffix has none.
    if psl::domain(domain.as_bytes()).is_none() {
        return Err(OrgError::Invalid("That is a public suffix, not an organization's domain."));
    }
    Ok(domain)
}

fn org_name(raw: &str) -> Result<String, OrgError> {
    let name = raw.trim();
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err(OrgError::Invalid("The name must be 1 to 64 characters."));
    }
    Ok(name.to_owned())
}

/// Creates an org: the tenant, its creator's membership as Org owner, and the creator's email domain, pending.
pub fn create_org(
    c: &mut DbConnInner,
    creator: &str,
    creator_email: &str,
    name: &str,
    tier: Tier,
    now: i64,
) -> Result<(String, ReinsOrgDomain), OrgError> {
    let name = org_name(name)?;
    let domain = email_domain(creator_email).ok_or(OrgError::Invalid("Your account's email has no domain."))?;
    let domain = claimable(&domain)?;
    c.transaction(|c| {
        if reins_org::q_verified_owner(c, &domain)?.is_some() {
            return Err(OrgError::Taken("Another organization has verified this domain."));
        }
        if !reins_org::q_orgs_of_user_any(c, creator)?.is_empty() {
            return Err(OrgError::Invalid("This account already belongs to an organization."));
        }
        let org = Organization::new(name.clone(), creator_email, None, None);
        let org_uuid = org.uuid.to_string();
        diesel::insert_into(organizations::table).values(&org).execute(c)?;
        reins_org::q_insert_org(
            c,
            &ReinsOrg {
                org_uuid: org_uuid.clone(),
                tier: tier.as_str().to_owned(),
                identity: IdentityKind::None.as_str().to_owned(),
                workos_org_id: None,
                directory_id: None,
                github_org: None,
                two_person: true,
                owner_recovery_at: None,
                created_by: creator.to_owned(),
                created_at: now,
                updated_at: now,
                body_retention_days: 30,
            },
        )?;
        let membership = Membership::new(creator.to_owned().into(), org.uuid.clone(), None);
        reins_org::q_add_member(c, &membership, Some(creator), now)?;
        reins_org::q_grant(
            c,
            &ReinsOrgRole {
                org_uuid: org_uuid.clone(),
                subject_kind: SUBJECT_MEMBER.to_owned(),
                subject_id: membership.uuid.to_string(),
                role: Role::OrgOwner.as_str().to_owned(),
                granted_at: now,
                granted_by: creator.to_owned(),
            },
        )?;
        let row = new_domain(&org_uuid, &domain, now);
        reins_org::q_insert_domain(c, &row)?;
        event(c, &org_uuid, "org.created", Some(creator), None, &json!({"name": name, "tier": tier}), now)?;
        event(
            c,
            &org_uuid,
            "member.joined",
            Some(creator),
            Some(membership.uuid.as_ref()),
            &json!({"role": "org_owner"}),
            now,
        )?;
        event(c, &org_uuid, "domain.added", Some(creator), Some(&domain), &json!({}), now)?;
        Ok((org_uuid, row))
    })
}

fn new_domain(org: &str, domain: &str, now: i64) -> ReinsOrgDomain {
    ReinsOrgDomain {
        org_uuid: org.to_owned(),
        domain: domain.to_owned(),
        token: random_token(),
        verified_at: None,
        checked_at: None,
        created_at: now,
    }
}

/// Adds a pending domain. Not a two-person action: a pending domain grants nothing until its owner proves it in DNS.
pub fn add_domain(
    c: &mut DbConnInner,
    org: &str,
    raw: &str,
    actor: &str,
    now: i64,
) -> Result<ReinsOrgDomain, OrgError> {
    require(c, org, actor, Capability::SetUpOrg)?;
    let domain = claimable(raw)?;
    c.transaction(|c| {
        if let Some(existing) = reins_org::q_domain(c, org, &domain)? {
            return Ok(existing);
        }
        if reins_org::q_domains(c, org)?.len() >= MAX_DOMAINS {
            return Err(OrgError::Invalid("This organization has as many domains as it can have."));
        }
        if reins_org::q_verified_owner(c, &domain)?.is_some() {
            return Err(OrgError::Taken("Another organization has verified this domain."));
        }
        let row = new_domain(org, &domain, now);
        reins_org::q_insert_domain(c, &row)?;
        event(c, org, "domain.added", Some(actor), Some(&domain), &json!({}), now)?;
        Ok(row)
    })
}

/// Whether `records` hold exactly the verification record for `token`.
#[must_use]
pub fn records_match(records: &[String], token: &str) -> bool {
    let want = record_value(token);
    records.iter().any(|r| crate::crypto::ct_eq(r.trim(), &want))
}

/// Checks the DNS record of a pending domain; `Ok(true)` once verified.
pub async fn verify_domain(
    conn: &DbConn,
    org: &str,
    raw: &str,
    actor: &str,
    resolver: &dyn TxtResolver,
    now: i64,
) -> Result<bool, OrgError> {
    let domain = normalize_domain(raw).ok_or(OrgError::NotFound)?;
    let (org_s, domain_s, actor_s) = (org.to_owned(), domain.clone(), actor.to_owned());
    let row = conn
        .run(move |c| {
            require(c, &org_s, &actor_s, Capability::SetUpOrg)?;
            reins_org::q_domain(c, &org_s, &domain_s)?.ok_or(OrgError::NotFound)
        })
        .await?;
    if row.verified_at.is_some() {
        return Ok(true);
    }
    let records = resolver.txt(&record_name(&domain)).await.map_err(|e| {
        warn!("Domain verification of {domain}: {e}");
        OrgError::Invalid("The DNS lookup failed. Try again in a minute.")
    })?;
    let matched = records_match(&records, &row.token);
    let (org, actor) = (org.to_owned(), actor.to_owned());
    conn.run(move |c| finish_verification(c, &org, &domain, &actor, matched, now)).await
}

fn finish_verification(
    c: &mut DbConnInner,
    org: &str,
    domain: &str,
    actor: &str,
    matched: bool,
    now: i64,
) -> Result<bool, OrgError> {
    if !matched {
        reins_org::q_mark_checked(c, org, domain, now)?;
        event(c, org, "domain.check_failed", Some(actor), Some(domain), &json!({}), now)?;
        return Ok(false);
    }
    if !reins_org::q_mark_verified(c, org, domain, now)? {
        return Err(OrgError::Taken("Another organization has verified this domain."));
    }
    event(c, org, "domain.verified", Some(actor), Some(domain), &json!({}), now)?;
    Ok(true)
}

#[cfg(all(test, sqlite))]
mod tests {
    use super::super::{can, dns::StaticTxt, testing::*};
    use super::*;
    use crate::db::models::reins_org_event::q_events;

    fn created(c: &mut DbConnInner) -> (String, ReinsOrgDomain) {
        add_user(c, "u1", "dana@acme.com");
        create_org(c, "u1", "dana@acme.com", "Acme", Tier::Business, 10).unwrap()
    }

    #[test]
    fn the_creator_becomes_owner_with_a_pending_domain() {
        let mut c = db();
        let (org, domain) = created(&mut c);
        assert_eq!(domain.domain, "acme.com");
        assert_eq!(domain.verified_at, None);
        assert_eq!(domain.token.len(), 43);
        assert!(can(&mut c, &org, "u1", Capability::SetUpOrg).unwrap());
        assert!(can(&mut c, &org, "u1", Capability::AssignAdmins).unwrap());
        let row = reins_org::q_org(&mut c, &org).unwrap().unwrap();
        assert_eq!((row.tier(), row.identity(), row.two_person), (Tier::Business, IdentityKind::None, true));
        let kinds: Vec<String> = q_events(&mut c, &org, None, 10).unwrap().into_iter().map(|e| e.kind).collect();
        assert_eq!(kinds, ["domain.added", "member.joined", "org.created"]);
    }

    #[test]
    fn names_and_public_suffixes_are_refused() {
        let mut c = db();
        add_user(&mut c, "u1", "x@co.uk");
        assert!(matches!(create_org(&mut c, "u1", "x@co.uk", "X", Tier::Team, 1), Err(OrgError::Invalid(_))));
        assert!(matches!(create_org(&mut c, "u1", "x@acme.com", " ", Tier::Team, 1), Err(OrgError::Invalid(_))));
        assert!(claimable("com").is_err());
        assert!(claimable("co.uk").is_err());
        assert!(claimable("github.io").is_err());
        assert_eq!(claimable("acme.co.uk").unwrap(), "acme.co.uk");
        assert_eq!(claimable("eng.acme.com").unwrap(), "eng.acme.com");
        assert_eq!(claimable("acme.github.io").unwrap(), "acme.github.io");
    }

    #[tokio::test]
    async fn verification_needs_the_exact_record() {
        let token = "abc";
        let want = record_value(token);
        assert!(records_match(std::slice::from_ref(&want), token));
        assert!(records_match(&["v=spf1 -all".to_owned(), format!(" {want} ")], token));
        assert!(!records_match(&[record_value("other")], token));
        assert!(!records_match(&[format!("x {want}")], token));
        assert!(!records_match(&[format!("{want}x")], token));
        assert!(!records_match(&[], token));
        // Answers come from exactly `_reins-challenge.<domain>`: a record on the parent or the bare domain is not it.
        let dns = StaticTxt::parse(&format!("acme.com={want};_reins-challenge.com={want}"));
        assert!(dns.txt(&record_name("acme.com")).await.unwrap().is_empty());
        let dns = StaticTxt::parse(&format!("_reins-challenge.acme.com.={want}"));
        assert_eq!(dns.txt(&record_name("acme.com")).await.unwrap(), [want]);
    }

    #[test]
    fn a_verified_domain_cannot_be_claimed_by_another_org() {
        let mut c = db();
        let (org, _) = created(&mut c);
        assert!(finish_verification(&mut c, &org, "acme.com", "u1", true, 20).unwrap());
        add_user(&mut c, "u2", "eve@acme.com");
        assert!(matches!(create_org(&mut c, "u2", "eve@acme.com", "Evil", Tier::Team, 30), Err(OrgError::Taken(_))));
        add_user(&mut c, "u3", "eve@evil.com");
        let (evil, _) = create_org(&mut c, "u3", "eve@evil.com", "Evil", Tier::Team, 30).unwrap();
        assert!(matches!(add_domain(&mut c, &evil, "acme.com", "u3", 31), Err(OrgError::Taken(_))));
    }

    #[test]
    fn a_failed_check_is_recorded_and_changes_nothing() {
        let mut c = db();
        let (org, _) = created(&mut c);
        assert!(!finish_verification(&mut c, &org, "acme.com", "u1", false, 20).unwrap());
        let d = reins_org::q_domain(&mut c, &org, "acme.com").unwrap().unwrap();
        assert_eq!((d.verified_at, d.checked_at), (None, Some(20)));
        assert_eq!(q_events(&mut c, &org, None, 1).unwrap()[0].kind, "domain.check_failed");
    }

    #[test]
    fn only_setup_capable_members_add_domains() {
        let mut c = db();
        let (org, _) = created(&mut c);
        add_member(&mut c, &org, "m2", "u2", reins_org::ACTIVE_STATUS);
        grant(&mut c, &org, SUBJECT_MEMBER, "m2", Role::SecurityAdmin);
        assert_eq!(add_domain(&mut c, &org, "acme.io", "u2", 11).unwrap_err(), OrgError::Forbidden);
        assert_eq!(add_domain(&mut c, &org, "acme.io", "nobody", 11).unwrap_err(), OrgError::Forbidden);
        let d = add_domain(&mut c, &org, "Acme.IO", "u1", 11).unwrap();
        assert_eq!(d.domain, "acme.io");
        // Adding it again gives the same pending row (same token).
        assert_eq!(add_domain(&mut c, &org, "acme.io", "u1", 12).unwrap().token, d.token);
    }
}
