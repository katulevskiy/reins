//! Reins for organizations, O1 (`docs/design/decisions-v2/05-organizations.md`): the Managed trust mode's tenant on
//! top of Vaultwarden's organizations (`db::models::reins_org`).
//!
//! - [`tenant`]: creating an org and verifying its domains (DNS TXT, [`dns`]).
//! - [`admin`]: roles (05 §3) and two-person admin changes.
//! - [`gate`]: who may sign in to a Managed account, and how (SSO only; Team: Google Workspace or a GitHub org).
//! - [`scim`]: members and groups from WorkOS Directory Sync, with the guards against a directory reaching outside its
//!   own tenant.
//! - [`deprovision`]: what removing a member does, in well under a minute.
//! - [`api`]: the Managed account's own view and device enrollment (phone API).
//! - [`console`]: the minimal admin console (server-rendered pages).
//!
//! Everything is off unless `REINS_ORGS_ENABLED` is set. An org never sees an account that is not its member: no
//! answer, page, event or error names one.
//!
//! **Tenant scope.** Each member's state in its org (`reins_org_members`: active, suspended or deprovisioned by the
//! directory, removed by an admin, suspended by the provider) is the org-side authority for sign-in, roles, owners and
//! recovery, and an account belongs to one org at most (a unique index): a Managed account is its org's alone, and a
//! Personal account is always a different account. Org code never writes `users.enabled`. It writes outside its own
//! rows only here:
//!
//! - creating the tenant (`organizations`) and its members' Vaultwarden memberships (`users_organizations`, `groups`,
//!   `groups_users` of its own org);
//! - SCIM: a stub account and invitation for a new address on one of its verified domains, when no account has it;
//! - deprovisioning (`deprovision::q_deprovision`, reached only for an account whose state row is in this org): the
//!   account's devices, sessions, roster and connections, and, for a directory delete, the account itself.

pub mod admin;
pub mod api;
pub mod attest;
pub mod audit;
pub mod console;
pub mod console_managed;
pub mod console_managed_pages;
pub mod console_pages;
pub mod decisions;
pub mod deprovision;
pub mod dns;
pub mod executor_api;
pub mod gate;
pub mod github;
pub mod invariants;
pub mod kms;
pub mod leases;
pub mod managed_mcp;
pub mod policy;
pub mod posture;
pub mod reviews;
pub mod roster;
pub mod routing;
pub mod scim;
pub mod sweeper;
pub mod tenant;
pub mod workos_api;

use reins_proto::org::{Capability, Role, role_grants};
use rocket::Route;

use crate::{
    CONFIG,
    db::{
        DbConnInner,
        models::{Membership, reins_org, reins_org_event},
    },
};

/// Whether organizations are on (`REINS_ORGS_ENABLED`, with Reins on).
pub fn enabled() -> bool {
    CONFIG.reins_enabled() && CONFIG.reins_orgs_enabled()
}

pub fn routes() -> Vec<Route> {
    if !enabled() {
        return Vec::new();
    }
    let mut routes = api::routes();
    routes.extend(console::routes());
    routes.extend(console_managed::routes());
    routes.extend(executor_api::routes());
    routes.extend(leases::routes());
    routes
}

/// Why an org operation was refused. The texts are shown to admins as they are, so they never name a non-member.
#[derive(Debug, PartialEq, Eq)]
pub enum OrgError {
    /// The caller may not do this (or is not a member).
    Forbidden,
    NotFound,
    Invalid(&'static str),
    /// Another org has it (a verified domain, a WorkOS organization).
    Taken(&'static str),
    Internal(String),
}

impl OrgError {
    #[must_use]
    pub fn message(&self) -> &str {
        match self {
            Self::Forbidden => "You can't do this in this organization.",
            Self::NotFound => "Not found.",
            Self::Invalid(m) | Self::Taken(m) => m,
            Self::Internal(_) => "Something went wrong. Try again.",
        }
    }
}

impl From<diesel::result::Error> for OrgError {
    fn from(e: diesel::result::Error) -> Self {
        Self::Internal(e.to_string())
    }
}

/// `user`'s membership in `org`, when they are an active member there (the org's own state, `reins_org_members`).
pub fn member_of(c: &mut DbConnInner, org: &str, user: &str) -> Result<Option<Membership>, OrgError> {
    if !reins_org::q_member_state(c, org, user)?.is_some_and(|m| m.is_active()) {
        return Ok(None);
    }
    Ok(reins_org::q_membership(c, org, user)?)
}

/// The roles `user` holds in `org` (none unless an active member there).
pub fn roles_of_user(c: &mut DbConnInner, org: &str, user: &str) -> Result<Vec<Role>, OrgError> {
    match member_of(c, org, user)? {
        Some(m) => Ok(reins_org::q_roles_of(c, org, m.uuid.as_ref())?),
        None => Ok(Vec::new()),
    }
}

/// Whether `user` may do `cap` in `org`.
pub fn can(c: &mut DbConnInner, org: &str, user: &str, cap: Capability) -> Result<bool, OrgError> {
    Ok(roles_of_user(c, org, user)?.into_iter().any(|r| role_grants(r, cap)))
}

/// Refuses unless `user` may do `cap` in `org`.
pub fn require(c: &mut DbConnInner, org: &str, user: &str, cap: Capability) -> Result<(), OrgError> {
    if can(c, org, user, cap)? {
        Ok(())
    } else {
        Err(OrgError::Forbidden)
    }
}

/// Appends an admin event (`reins_org_events`).
pub fn event(
    c: &mut DbConnInner,
    org: &str,
    kind: &str,
    actor: Option<&str>,
    subject: Option<&str>,
    detail: &serde_json::Value,
    now: i64,
) -> Result<(), OrgError> {
    reins_org_event::append(c, org, kind, actor, subject, detail, now)?;
    Ok(())
}

/// A fresh random token (base64url, 32 bytes).
pub fn random_token() -> String {
    crate::crypto::encode_random_bytes::<32>(&data_encoding::BASE64URL_NOPAD)
}

#[cfg(all(test, sqlite))]
pub(crate) mod testing {
    use crate::db::DbConnInner;
    #[allow(unused_imports)]
    pub use crate::db::models::reins_org::tests::{add_group, add_member, add_org, grant};

    pub fn db() -> DbConnInner {
        crate::db::models::reins_device_test_db()
    }

    /// An enabled account with `email` (if `uuid` has none yet).
    pub fn add_user(c: &mut DbConnInner, uuid: &str, email: &str) {
        crate::db::models::reins_org::tests::ensure_user(c, uuid, email);
    }

    /// An active member `user` of `org` (membership `m-<user>`) holding `roles`; returns the membership id.
    pub fn member(c: &mut DbConnInner, org: &str, user: &str, roles: &[reins_proto::org::Role]) -> String {
        let m = format!("m-{user}");
        add_member(c, org, &m, user, crate::db::models::reins_org::ACTIVE_STATUS);
        for r in roles {
            grant(c, org, crate::db::models::reins_org::SUBJECT_MEMBER, &m, *r);
        }
        m
    }

    /// Sets a member's state in `org`.
    pub fn set_state(c: &mut DbConnInner, org: &str, user: &str, state: &str) {
        crate::db::models::reins_org::q_set_state(c, org, user, state, None, 1).unwrap();
    }
}
