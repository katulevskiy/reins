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

#![allow(dead_code)] // O1 wiring in progress: the routes and console use these.

pub mod admin;
pub mod deprovision;
pub mod dns;
pub mod gate;
pub mod github;
pub mod invariants;
pub mod scim;
pub mod tenant;
pub mod workos_api;

use reins_proto::org::{Capability, Role, role_grants};
use rocket::Route;

use crate::{
    CONFIG,
    db::{
        DbConnInner,
        models::{
            Membership,
            reins_org::{self, ACTIVE_STATUS},
            reins_org_event,
        },
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
    Vec::new()
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

/// `user`'s active membership in `org`.
pub fn member_of(c: &mut DbConnInner, org: &str, user: &str) -> Result<Option<Membership>, OrgError> {
    Ok(reins_org::q_membership(c, org, user)?.filter(|m| m.status >= ACTIVE_STATUS))
}

/// The roles `user` holds in `org`: none unless an active member with an enabled account.
pub fn roles_of_user(c: &mut DbConnInner, org: &str, user: &str) -> Result<Vec<Role>, OrgError> {
    use diesel::prelude::*;
    let enabled: Option<bool> = crate::db::schema::users::table
        .filter(crate::db::schema::users::uuid.eq(user))
        .select(crate::db::schema::users::enabled)
        .first(c)
        .optional()?;
    if enabled != Some(true) {
        return Ok(Vec::new());
    }
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
    #[allow(unused_imports)]
    pub use crate::db::models::reins_org::tests::{add_group, add_member, add_org, grant};
    use crate::db::{DbConnInner, schema::users};
    use diesel::prelude::*;

    pub fn db() -> DbConnInner {
        crate::db::models::reins_device_test_db()
    }

    /// An enabled account with `email` (if `uuid` has none yet).
    pub fn add_user(c: &mut DbConnInner, uuid: &str, email: &str) {
        crate::db::models::reins_org::tests::ensure_user(c, uuid, email);
    }

    /// Disables (or enables) an account.
    pub fn set_enabled(c: &mut DbConnInner, uuid: &str, enabled: bool) {
        diesel::update(users::table.filter(users::uuid.eq(uuid))).set(users::enabled.eq(enabled)).execute(c).unwrap();
    }
}
