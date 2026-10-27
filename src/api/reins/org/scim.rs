//! Members and groups from the company's directory (05 §2: "SCIM: users and groups sync from the IdP"), through
//! WorkOS Directory Sync. Like the WorkOS user sync (`workos_sync`), a background task reads the Events API
//! (`dsync.*`) after a cursor kept in `reins_settings`, every `REINS_WORKOS_DSYNC_SECS` and at once when a signed
//! webhook arrives. A webhook body is never trusted: only events read from WorkOS with the server's key are applied.
//!
//! Guards against a directory reaching outside its own tenant (SCIM spoofing):
//!
//! - an event is applied only to the tenant whose **verified binding** names both its directory and its WorkOS
//!   organization (`org::workos_api`); events of an unbound directory are dropped;
//! - a user's email must be on one of that tenant's verified domains, or nothing happens (`scim.refused`);
//! - an email that already belongs to an account that is not the tenant's member is never bound to it (that would
//!   capture someone's Personal account); nothing the org can see mentions it;
//! - removals go through `deprovision`, and with it the last-owner rule (`org::invariants`).

use std::{sync::LazyLock, time::Duration};

use diesel::prelude::*;
use serde_json::{Value, json};
use tokio::sync::Notify;

use super::{
    OrgError,
    deprovision::{self, Reason, Removal},
    event,
};
use crate::{
    CONFIG,
    api::reins::workos_sync::Event,
    db::{
        DbConn, DbConnInner, DbPool,
        models::{
            Group, Invitation, Membership, ReinsSetting, User,
            reins_org::{self, ReinsOrg, SUBJECT_GROUP, state},
        },
        schema::{groups, groups_users, invitations, users, users_organizations},
    },
};

/// The events applied.
pub const EVENTS: &str = "dsync.user.created,dsync.user.updated,dsync.user.deleted,dsync.group.created,\
dsync.group.updated,dsync.group.deleted,dsync.group.user_added,dsync.group.user_removed,dsync.deleted";
/// `reins_settings` name of the cursor.
const CURSOR: &str = "workos.dsync.after";
/// Events per page.
const PAGE: usize = 100;

static NUDGE: LazyLock<Notify> = LazyLock::new(Notify::new);

/// Wakes the directory sync now (a signed WorkOS webhook arrived).
pub fn nudge() {
    NUDGE.notify_one();
}

/// Whether the directory sync runs: orgs on, WorkOS as the SSO provider, a non-zero interval.
pub fn enabled() -> bool {
    super::enabled() && CONFIG.sso_enabled() && crate::sso_workos::enabled() && CONFIG.reins_workos_dsync_secs() > 0
}

/// One directory event, as Reins applies it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DirAction {
    UpsertUser {
        directory_id: String,
        organization_id: Option<String>,
        user_id: String,
        email: String,
        name: Option<String>,
        active: bool,
    },
    DeleteUser {
        directory_id: String,
        organization_id: Option<String>,
        user_id: String,
    },
    UpsertGroup {
        directory_id: String,
        organization_id: Option<String>,
        group_id: String,
        name: String,
    },
    DeleteGroup {
        directory_id: String,
        organization_id: Option<String>,
        group_id: String,
    },
    AddToGroup {
        directory_id: String,
        organization_id: Option<String>,
        user_id: String,
        group_id: String,
    },
    RemoveFromGroup {
        directory_id: String,
        organization_id: Option<String>,
        user_id: String,
        group_id: String,
    },
    DirectoryDeleted {
        directory_id: String,
        organization_id: Option<String>,
    },
    Ignore,
}

fn str_at(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned)
}

/// A directory user's email: the primary one, else the first, else `email`.
fn user_email(user: &Value) -> Option<String> {
    let emails = user.get("emails").and_then(Value::as_array);
    let primary = emails
        .and_then(|e| e.iter().find(|x| x.get("primary").and_then(Value::as_bool) == Some(true)).or_else(|| e.first()));
    primary.and_then(|e| str_at(e, "value")).or_else(|| str_at(user, "email")).map(|e| e.trim().to_lowercase())
}

fn user_name(user: &Value) -> Option<String> {
    let name =
        [str_at(user, "first_name"), str_at(user, "last_name")].into_iter().flatten().collect::<Vec<_>>().join(" ");
    (!name.trim().is_empty()).then_some(name)
}

/// Reads one event.
pub fn plan(event: &Event) -> DirAction {
    let d = &event.data;
    let org_of = |v: &Value| str_at(v, "organization_id").or_else(|| str_at(d, "organization_id"));
    let dir_of = |v: &Value| str_at(v, "directory_id").or_else(|| str_at(d, "directory_id"));
    match event.event.as_str() {
        "dsync.user.created" | "dsync.user.updated" => {
            let (Some(directory_id), Some(user_id), Some(email)) = (dir_of(d), str_at(d, "id"), user_email(d)) else {
                return DirAction::Ignore;
            };
            DirAction::UpsertUser {
                directory_id,
                organization_id: org_of(d),
                user_id,
                email,
                name: user_name(d),
                active: str_at(d, "state").as_deref() == Some("active"),
            }
        }
        "dsync.user.deleted" => match (dir_of(d), str_at(d, "id")) {
            (Some(directory_id), Some(user_id)) => DirAction::DeleteUser {
                directory_id,
                organization_id: org_of(d),
                user_id,
            },
            _ => DirAction::Ignore,
        },
        "dsync.group.created" | "dsync.group.updated" => match (dir_of(d), str_at(d, "id"), str_at(d, "name")) {
            (Some(directory_id), Some(group_id), Some(name)) => DirAction::UpsertGroup {
                directory_id,
                organization_id: org_of(d),
                group_id,
                name: name.chars().take(100).collect(),
            },
            _ => DirAction::Ignore,
        },
        "dsync.group.deleted" => match (dir_of(d), str_at(d, "id")) {
            (Some(directory_id), Some(group_id)) => DirAction::DeleteGroup {
                directory_id,
                organization_id: org_of(d),
                group_id,
            },
            _ => DirAction::Ignore,
        },
        "dsync.group.user_added" | "dsync.group.user_removed" => {
            let (Some(user), Some(group)) = (d.get("user"), d.get("group")) else {
                return DirAction::Ignore;
            };
            let (Some(directory_id), Some(user_id), Some(group_id)) =
                (dir_of(d).or_else(|| dir_of(group)), str_at(user, "id"), str_at(group, "id"))
            else {
                return DirAction::Ignore;
            };
            let organization_id = org_of(group).or_else(|| org_of(user));
            if event.event == "dsync.group.user_added" {
                DirAction::AddToGroup {
                    directory_id,
                    organization_id,
                    user_id,
                    group_id,
                }
            } else {
                DirAction::RemoveFromGroup {
                    directory_id,
                    organization_id,
                    user_id,
                    group_id,
                }
            }
        }
        "dsync.deleted" => match str_at(d, "id").or_else(|| dir_of(d)) {
            Some(directory_id) => DirAction::DirectoryDeleted {
                directory_id,
                organization_id: org_of(d),
            },
            None => DirAction::Ignore,
        },
        _ => DirAction::Ignore,
    }
}

impl DirAction {
    fn binding(&self) -> Option<(&str, Option<&str>)> {
        match self {
            Self::UpsertUser {
                directory_id,
                organization_id,
                ..
            }
            | Self::DeleteUser {
                directory_id,
                organization_id,
                ..
            }
            | Self::UpsertGroup {
                directory_id,
                organization_id,
                ..
            }
            | Self::DeleteGroup {
                directory_id,
                organization_id,
                ..
            }
            | Self::AddToGroup {
                directory_id,
                organization_id,
                ..
            }
            | Self::RemoveFromGroup {
                directory_id,
                organization_id,
                ..
            }
            | Self::DirectoryDeleted {
                directory_id,
                organization_id,
            } => Some((directory_id, organization_id.as_deref())),
            Self::Ignore => None,
        }
    }
}

/// The tenant an event belongs to: the one bound to its directory **and** its WorkOS organization, on a tier with SCIM.
fn tenant_for(c: &mut DbConnInner, action: &DirAction) -> Result<Option<ReinsOrg>, OrgError> {
    let Some((directory, organization)) = action.binding() else {
        return Ok(None);
    };
    let Some(tenant) = reins_org::q_org_by_directory(c, directory)? else {
        return Ok(None);
    };
    let bound = organization.is_some() && tenant.workos_org_id.as_deref() == organization && tenant.tier().scim();
    Ok(bound.then_some(tenant))
}

fn verified_domains(c: &mut DbConnInner, org: &str) -> Result<Vec<String>, OrgError> {
    Ok(reins_org::q_domains(c, org)?.into_iter().filter(|d| d.verified_at.is_some()).map(|d| d.domain).collect())
}

fn membership_by_external(c: &mut DbConnInner, org: &str, user_id: &str) -> Result<Option<Membership>, OrgError> {
    Ok(users_organizations::table
        .filter(users_organizations::org_uuid.eq(org))
        .filter(users_organizations::external_id.eq(user_id))
        .first(c)
        .optional()?)
}

fn group_by_external(c: &mut DbConnInner, org: &str, group_id: &str) -> Result<Option<String>, OrgError> {
    Ok(groups::table
        .filter(groups::organizations_uuid.eq(org))
        .filter(groups::external_id.eq(group_id))
        .select(groups::uuid)
        .first(c)
        .optional()?)
}

/// Applies one action in the caller's transaction; returns the removals to finish after the commit.
pub fn apply(c: &mut DbConnInner, action: &DirAction, now: i64) -> Result<Vec<Removal>, OrgError> {
    let Some(tenant) = tenant_for(c, action)? else {
        if action.binding().is_some() {
            debug!("Directory sync: dropped an event of an unbound directory");
        }
        return Ok(Vec::new());
    };
    let org = tenant.org_uuid.as_str();
    match action {
        DirAction::UpsertUser {
            user_id,
            email,
            name,
            active,
            ..
        } => upsert_user(c, org, user_id, email, name.as_deref(), *active, now),
        DirAction::DeleteUser {
            user_id,
            ..
        } => match membership_by_external(c, org, user_id)? {
            Some(m) => {
                Ok(deprovision::q_deprovision(c, org, m.user_uuid.as_ref(), Reason::DirectoryDelete, None, now)?
                    .into_iter()
                    .collect())
            }
            None => Ok(Vec::new()),
        },
        DirAction::UpsertGroup {
            group_id,
            name,
            ..
        } => {
            if let Some(uuid) = group_by_external(c, org, group_id)? {
                diesel::update(groups::table.filter(groups::uuid.eq(&uuid)))
                    .set((groups::name.eq(name), groups::revision_date.eq(chrono::Utc::now().naive_utc())))
                    .execute(c)?;
            } else {
                let g = Group::new(org.to_owned().into(), name.clone(), false, Some(group_id.clone()));
                diesel::insert_into(groups::table).values(&g).execute(c)?;
                event(c, org, "group.provisioned", None, Some(g.uuid.as_ref()), &json!({"name": name}), now)?;
            }
            Ok(Vec::new())
        }
        DirAction::DeleteGroup {
            group_id,
            ..
        } => {
            if let Some(uuid) = group_by_external(c, org, group_id)? {
                diesel::delete(groups_users::table.filter(groups_users::groups_uuid.eq(&uuid))).execute(c)?;
                reins_org::q_revoke_all(c, org, SUBJECT_GROUP, &uuid)?;
                diesel::delete(groups::table.filter(groups::uuid.eq(&uuid))).execute(c)?;
                event(c, org, "group.deprovisioned", None, Some(&uuid), &json!({}), now)?;
            }
            Ok(Vec::new())
        }
        DirAction::AddToGroup {
            user_id,
            group_id,
            ..
        }
        | DirAction::RemoveFromGroup {
            user_id,
            group_id,
            ..
        } => {
            let (Some(m), Some(g)) = (membership_by_external(c, org, user_id)?, group_by_external(c, org, group_id)?)
            else {
                return Ok(Vec::new());
            };
            let m = m.uuid.to_string();
            diesel::delete(
                groups_users::table
                    .filter(groups_users::groups_uuid.eq(&g))
                    .filter(groups_users::users_organizations_uuid.eq(&m)),
            )
            .execute(c)?;
            let added = matches!(action, DirAction::AddToGroup { .. });
            if added {
                diesel::insert_into(groups_users::table)
                    .values((groups_users::groups_uuid.eq(&g), groups_users::users_organizations_uuid.eq(&m)))
                    .execute(c)?;
            }
            let kind = if added {
                "group.member_added"
            } else {
                "group.member_removed"
            };
            event(c, org, kind, None, Some(&g), &json!({"member": m}), now)?;
            Ok(Vec::new())
        }
        DirAction::DirectoryDeleted {
            ..
        } => {
            let mut t = tenant.clone();
            t.directory_id = None;
            t.updated_at = now;
            reins_org::q_save_org(c, &t)?;
            event(c, org, "directory.unbound", None, None, &json!({}), now)?;
            Ok(Vec::new())
        }
        DirAction::Ignore => Ok(Vec::new()),
    }
}

fn upsert_user(
    c: &mut DbConnInner,
    org: &str,
    user_id: &str,
    email: &str,
    name: Option<&str>,
    active: bool,
    now: i64,
) -> Result<Vec<Removal>, OrgError> {
    // A member the directory already linked: its word on suspension always applies, whatever the address now says.
    if let Some(m) = membership_by_external(c, org, user_id)? {
        let user = m.user_uuid.to_string();
        let Some(member) = reins_org::q_member_state(c, org, &user)? else {
            return Ok(Vec::new());
        };
        if !active {
            return Ok(deprovision::q_deprovision(c, org, &user, Reason::DirectorySuspend, None, now)?
                .into_iter()
                .collect());
        }
        // Back in the directory: only what the directory itself suspended comes back. An admin's removal needs an
        // admin; a deprovisioned account is gone. Roles ended with the membership stay ended; a last owner kept
        // through recovery is an owner again.
        if member.state == state::SUSPENDED_BY_DIRECTORY {
            reins_org::q_set_state(c, org, &user, state::ACTIVE, None, now)?;
            event(c, org, "member.restored", None, Some(m.uuid.as_ref()), &json!({}), now)?;
            if super::invariants::refresh_owner_recovery(c, org)? {
                event(c, org, "org.owner_recovered", None, Some(m.uuid.as_ref()), &json!({"by_directory": true}), now)?;
            }
        }
        return Ok(Vec::new());
    }
    if !active {
        return Ok(Vec::new());
    }
    // Creating or linking a membership: only for an address on one of the tenant's verified domains.
    let domain_ok =
        reins_proto::org::email_domain(email).is_some_and(|d| verified_domains(c, org).is_ok_and(|v| v.contains(&d)));
    if !domain_ok {
        // The directory's own data, about its own user id: no Reins account is named.
        event(c, org, "scim.refused", None, Some(user_id), &json!({"why": "email_outside_verified_domains"}), now)?;
        return Ok(Vec::new());
    }
    let existing: Option<String> =
        users::table.filter(users::email.eq(email)).select(users::uuid).first(c).optional()?;
    if let Some(user) = existing {
        match (reins_org::q_membership(c, org, &user)?, reins_org::q_member_state(c, org, &user)?) {
            // A member who joined before the directory (the creator, a Team-era member): link them.
            (Some(m), Some(st)) if st.is_active() && m.external_id.is_none() => {
                diesel::update(users_organizations::table.filter(users_organizations::uuid.eq(m.uuid.to_string())))
                    .set(users_organizations::external_id.eq(user_id))
                    .execute(c)?;
                event(c, org, "member.linked", None, Some(m.uuid.as_ref()), &json!({}), now)?;
            }
            (Some(_), _) => {}
            (None, _) => {
                // Someone's own account on this address, or another org's member: never captured, never mentioned
                // to this org.
                info!("Directory sync: org {org} provisions an address that has another account; left alone");
            }
        }
    } else {
        let mut stub = User::new(email, name.map(str::to_owned));
        stub.verified_at = None;
        diesel::insert_into(users::table).values(&stub).execute(c)?;
        let invited: i64 = invitations::table.filter(invitations::email.eq(email)).count().get_result(c)?;
        if invited == 0 {
            diesel::insert_into(invitations::table).values(&Invitation::new(email)).execute(c)?;
        }
        let mut m = Membership::new(stub.uuid.clone(), org.to_owned().into(), None);
        m.external_id = Some(user_id.to_owned());
        reins_org::q_add_member(c, &m, None, now)?;
        event(c, org, "member.provisioned", None, Some(m.uuid.as_ref()), &json!({}), now)?;
    }
    Ok(Vec::new())
}

/// Where the events come from.
#[rocket::async_trait]
pub trait EventSource: Send + Sync {
    async fn page(&self, after: Option<&str>) -> Result<Vec<Event>, String>;
}

/// The WorkOS Events API, with the server's key.
pub struct WorkosEvents;

#[rocket::async_trait]
impl EventSource for WorkosEvents {
    async fn page(&self, after: Option<&str>) -> Result<Vec<Event>, String> {
        let base = crate::sso_workos::api_base().map_err(|e| format!("{e:?}"))?;
        let mut url = url::Url::parse(&format!("{base}/events")).map_err(|e| e.to_string())?;
        url.query_pairs_mut()
            .append_pair("events", EVENTS)
            .append_pair("limit", &PAGE.to_string())
            .append_pair("order", "asc");
        if let Some(after) = after {
            url.query_pairs_mut().append_pair("after", after);
        }
        let client = crate::sso_workos::http_client().map_err(|e| format!("{e:?}"))?;
        let response = client
            .get(url)
            .bearer_auth(crate::sso_workos::api_key())
            .send()
            .await
            .map_err(|e| format!("could not reach WorkOS: {e}"))?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("WorkOS answered {status}"));
        }
        let page: crate::api::reins::workos_sync::EventsPage =
            response.json().await.map_err(|e| format!("unexpected WorkOS events answer: {e}"))?;
        Ok(page.data)
    }
}

/// Reads and applies every event after the cursor; returns how many. The cursor moves after each applied event, so a
/// failure retries from the event that failed.
pub async fn sync_once(source: &dyn EventSource, conn: &DbConn, pool: Option<&DbPool>) -> Result<usize, String> {
    let mut cursor = ReinsSetting::get(CURSOR, conn).await.map_err(|e| e.to_string())?;
    let mut applied = 0;
    loop {
        let page = source.page(cursor.as_deref()).await?;
        let full = page.len() >= PAGE;
        for ev in page {
            let action = plan(&ev);
            let now = super::super::now_unix();
            let id = ev.id.clone();
            let removals = conn
                .run(move |c| c.transaction(|c| apply(c, &action, now)))
                .await
                .map_err(|e| format!("event {id}: {e:?}"))?;
            for removal in removals {
                deprovision::finish(removal, pool).await;
            }
            ReinsSetting::set(CURSOR, &ev.id, conn).await.map_err(|e| e.to_string())?;
            cursor = Some(ev.id);
            applied += 1;
        }
        if !full {
            return Ok(applied);
        }
    }
}

/// How often every bound directory is listed again, besides right after it is bound.
const BACKFILL_EVERY: Duration = Duration::from_secs(30 * 60);

static BACKFILL_NOW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Lists every bound directory at the next tick (a directory was just bound: its events before the binding were
/// dropped, so its users and groups come from listing it).
pub fn backfill_soon() {
    BACKFILL_NOW.store(true, std::sync::atomic::Ordering::SeqCst);
    nudge();
}

/// A directory's users and groups as WorkOS lists them now.
#[rocket::async_trait]
pub trait DirectoryListing: Send + Sync {
    async fn users(&self, directory: &str) -> Result<Vec<Value>, String>;
    async fn groups(&self, directory: &str) -> Result<Vec<Value>, String>;
}

/// `GET /directory_users` and `GET /directory_groups` with the server's key, every page.
pub struct WorkosListing;

async fn list_all(path: &str, directory: &str) -> Result<Vec<Value>, String> {
    let base = crate::sso_workos::api_base().map_err(|e| format!("{e:?}"))?;
    let client = crate::sso_workos::http_client().map_err(|e| format!("{e:?}"))?;
    let mut out = Vec::new();
    let mut after: Option<String> = None;
    for _ in 0..1_000 {
        let mut url = url::Url::parse(&format!("{base}/{path}")).map_err(|e| e.to_string())?;
        url.query_pairs_mut().append_pair("directory", directory).append_pair("limit", "100");
        if let Some(a) = &after {
            url.query_pairs_mut().append_pair("after", a);
        }
        let response = client
            .get(url)
            .bearer_auth(crate::sso_workos::api_key())
            .send()
            .await
            .map_err(|e| format!("could not reach WorkOS: {e}"))?;
        if !response.status().is_success() {
            return Err(format!("WorkOS answered {}", response.status()));
        }
        let page: Value = response.json().await.map_err(|e| format!("unexpected WorkOS list: {e}"))?;
        out.extend(page["data"].as_array().cloned().unwrap_or_default());
        after = page["list_metadata"]["after"].as_str().map(str::to_owned);
        if after.is_none() {
            return Ok(out);
        }
    }
    Err("the directory listing did not end".to_owned())
}

#[rocket::async_trait]
impl DirectoryListing for WorkosListing {
    async fn users(&self, directory: &str) -> Result<Vec<Value>, String> {
        list_all("directory_users", directory).await
    }

    async fn groups(&self, directory: &str) -> Result<Vec<Value>, String> {
        list_all("directory_groups", directory).await
    }
}

/// Applies a complete listing of `tenant`'s directory: its groups, its users and their group memberships, as the
/// events would have; then the members it linked before whose directory id is gone are deprovisioned (the directory
/// deleted them while nothing listened). One transaction.
pub fn q_backfill(
    c: &mut DbConnInner,
    tenant: &ReinsOrg,
    users: &[Value],
    groups: &[Value],
    now: i64,
) -> Result<Vec<Removal>, OrgError> {
    let (Some(directory), Some(workos)) = (tenant.directory_id.as_deref(), tenant.workos_org_id.as_deref()) else {
        return Ok(Vec::new());
    };
    let event = |kind: &str, data: &Value| {
        let mut data = data.clone();
        data["directory_id"] = json!(directory);
        data["organization_id"] = json!(workos);
        Event {
            id: String::new(),
            event: kind.to_owned(),
            data,
        }
    };
    let mut removals = Vec::new();
    for g in groups {
        removals.extend(apply(c, &plan(&event("dsync.group.updated", g)), now)?);
    }
    let mut listed = std::collections::HashSet::new();
    for u in users {
        if let Some(id) = u.get("id").and_then(Value::as_str) {
            listed.insert(id.to_owned());
        }
        removals.extend(apply(c, &plan(&event("dsync.user.updated", u)), now)?);
        let user_id = u.get("id").and_then(Value::as_str).unwrap_or_default();
        for g in u.get("groups").and_then(Value::as_array).cloned().unwrap_or_default() {
            if let Some(group_id) = g.get("id").and_then(Value::as_str) {
                let added =
                    event("dsync.group.user_added", &json!({"user": {"id": user_id}, "group": {"id": group_id}}));
                removals.extend(apply(c, &plan(&added), now)?);
            }
        }
    }
    let org = tenant.org_uuid.as_str();
    for m in reins_org::q_members(c, org)? {
        let Some(external) = m.external_id.as_deref() else {
            continue;
        };
        let user = m.user_uuid.to_string();
        let active = reins_org::q_member_state(c, org, &user)?.is_some_and(|s| s.is_active());
        if active && !listed.contains(external) {
            removals.extend(deprovision::q_deprovision(c, org, &user, Reason::DirectoryDelete, None, now)?);
        }
    }
    Ok(removals)
}

/// Lists and applies every bound directory. A listing that fails changes nothing for that tenant.
pub async fn backfill_all(
    listing: &dyn DirectoryListing,
    conn: &DbConn,
    pool: Option<&DbPool>,
) -> Result<usize, String> {
    let tenants = conn.run(|c| reins_org::q_bound_directories(c)).await.map_err(|e| e.to_string())?;
    let mut done = 0;
    for tenant in tenants.into_iter().filter(|t| t.tier().scim()) {
        let Some(directory) = tenant.directory_id.clone() else {
            continue;
        };
        let (users, groups) = match (listing.users(&directory).await, listing.groups(&directory).await) {
            (Ok(u), Ok(g)) => (u, g),
            (Err(e), _) | (_, Err(e)) => {
                warn!("Directory backfill of org {}: {e}", tenant.org_uuid);
                continue;
            }
        };
        let now = super::super::now_unix();
        let removals = conn
            .run(move |c| c.transaction(|c| q_backfill(c, &tenant, &users, &groups, now)))
            .await
            .map_err(|e| format!("{e:?}"))?;
        for removal in removals {
            deprovision::finish(removal, pool).await;
        }
        done += 1;
    }
    Ok(done)
}

/// Starts the background task (once, inside the runtime).
pub fn spawn(pool: DbPool) {
    if !enabled() {
        return;
    }
    let every = Duration::from_secs(CONFIG.reins_workos_dsync_secs());
    info!("Directory sync: reading WorkOS directory events every {}s", every.as_secs());
    tokio::spawn(async move {
        let mut last_backfill = tokio::time::Instant::now();
        loop {
            match pool.get().await {
                Ok(conn) => {
                    match sync_once(&WorkosEvents, &conn, Some(&pool)).await {
                        Ok(0) => {}
                        Ok(n) => info!("Directory sync: applied {n} events"),
                        Err(e) => warn!("Directory sync failed: {e}"),
                    }
                    if BACKFILL_NOW.swap(false, std::sync::atomic::Ordering::SeqCst)
                        || last_backfill.elapsed() >= BACKFILL_EVERY
                    {
                        last_backfill = tokio::time::Instant::now();
                        if let Err(e) = backfill_all(&WorkosListing, &conn, Some(&pool)).await {
                            warn!("Directory backfill failed: {e}");
                        }
                    }
                    // Deprovisioned accounts' erasure, also when the user sync is off.
                    if let Err(e) = crate::api::reins::workos_sync::drain_deletions(&conn).await {
                        warn!("Directory sync: deletions deferred: {e}");
                    }
                }
                Err(e) => warn!("Directory sync: no database connection: {e:?}"),
            }
            tokio::select! {
                () = tokio::time::sleep(every) => {}
                () = NUDGE.notified() => {}
            }
        }
    });
}

#[cfg(all(test, sqlite))]
mod tests {
    use serde_json::json;

    use super::super::testing::*;
    use super::*;
    use crate::db::models::reins_org::{ACTIVE_STATUS, q_save_org};

    fn ev(kind: &str, data: Value) -> Event {
        Event {
            id: format!("event_{kind}"),
            event: kind.to_owned(),
            data,
        }
    }

    fn dir_user(id: &str, email: &str, state: &str) -> Value {
        json!({"object": "directory_user", "id": id, "directory_id": "directory_ACME", "organization_id": "org_ACME",
               "idp_id": "123", "first_name": "Dana", "last_name": "Lee", "state": state,
               "emails": [{"primary": false, "type": "home", "value": "dana@home.example"},
                          {"primary": true, "type": "work", "value": email}]})
    }

    fn bound_tenant(c: &mut DbConnInner) {
        add_org(c, "o1");
        let mut t = reins_org::q_org(c, "o1").unwrap().unwrap();
        t.workos_org_id = Some("org_ACME".to_owned());
        t.directory_id = Some("directory_ACME".to_owned());
        q_save_org(c, &t).unwrap();
        reins_org::q_insert_domain(
            c,
            &reins_org::ReinsOrgDomain {
                org_uuid: "o1".to_owned(),
                domain: "acme.com".to_owned(),
                token: "t".to_owned(),
                verified_at: None,
                checked_at: None,
                created_at: 1,
            },
        )
        .unwrap();
        assert!(reins_org::q_mark_verified(c, "o1", "acme.com", 1).unwrap());
    }

    fn run(c: &mut DbConnInner, e: &Event) -> Vec<Removal> {
        apply(c, &plan(e), 10).unwrap()
    }

    fn member_by_email(c: &mut DbConnInner, email: &str) -> Option<Membership> {
        let user: Option<String> =
            users::table.filter(users::email.eq(email)).select(users::uuid).first(c).optional().unwrap();
        user.and_then(|u| reins_org::q_membership(c, "o1", &u).unwrap())
    }

    #[test]
    fn plans_read_directory_events() {
        let a = plan(&ev("dsync.user.created", dir_user("directory_user_1", "Dana@Acme.com", "active")));
        assert_eq!(
            a,
            DirAction::UpsertUser {
                directory_id: "directory_ACME".to_owned(),
                organization_id: Some("org_ACME".to_owned()),
                user_id: "directory_user_1".to_owned(),
                email: "dana@acme.com".to_owned(),
                name: Some("Dana Lee".to_owned()),
                active: true,
            }
        );
        let added = plan(&ev(
            "dsync.group.user_added",
            json!({"directory_id": "directory_ACME",
                   "user": dir_user("directory_user_1", "dana@acme.com", "active"),
                   "group": {"object": "directory_group", "id": "directory_group_1", "name": "Security",
                             "directory_id": "directory_ACME", "organization_id": "org_ACME"}}),
        ));
        assert!(matches!(added, DirAction::AddToGroup { ref group_id, .. } if group_id == "directory_group_1"));
        assert!(matches!(
            plan(&ev("dsync.user.updated", dir_user("u", "x@acme.com", "inactive"))),
            DirAction::UpsertUser {
                active: false,
                ..
            }
        ));
        assert_eq!(plan(&ev("dsync.user.created", json!({"id": "u"}))), DirAction::Ignore);
        assert_eq!(plan(&ev("user.created", json!({}))), DirAction::Ignore);
    }

    #[test]
    fn a_directory_cannot_provision_outside_its_verified_domains() {
        let mut c = db();
        bound_tenant(&mut c);
        add_user(&mut c, "ceo", "ceo@other.com");
        run(&mut c, &ev("dsync.user.created", dir_user("directory_user_9", "ceo@other.com", "active")));
        run(&mut c, &ev("dsync.user.created", dir_user("directory_user_8", "new@other.com", "active")));
        assert!(reins_org::q_membership(&mut c, "o1", "ceo").unwrap().is_none());
        assert!(member_by_email(&mut c, "new@other.com").is_none());
        let n: i64 = users::table.filter(users::email.eq("new@other.com")).count().get_result(&mut c).unwrap();
        assert_eq!(n, 0, "no stub account either");
        let e = crate::db::models::reins_org_event::q_events(&mut c, "o1", None, 1).unwrap().remove(0);
        assert_eq!(e.kind, "scim.refused");
        assert!(!e.detail.contains("other.com"), "{}", e.detail);
    }

    #[test]
    fn events_for_an_unbound_directory_are_ignored() {
        let mut c = db();
        bound_tenant(&mut c);
        // Another WorkOS organization naming our directory, our organization with another directory, and none.
        let mut spoofed = dir_user("directory_user_1", "dana@acme.com", "active");
        spoofed["organization_id"] = json!("org_EVIL");
        run(&mut c, &ev("dsync.user.created", spoofed));
        let mut other_dir = dir_user("directory_user_1", "dana@acme.com", "active");
        other_dir["directory_id"] = json!("directory_EVIL");
        run(&mut c, &ev("dsync.user.created", other_dir));
        let mut no_org = dir_user("directory_user_1", "dana@acme.com", "active");
        no_org.as_object_mut().unwrap().remove("organization_id");
        run(&mut c, &ev("dsync.user.created", no_org));
        assert!(member_by_email(&mut c, "dana@acme.com").is_none());
        // A Team tenant has no SCIM, even when bound.
        let mut t = reins_org::q_org(&mut c, "o1").unwrap().unwrap();
        t.tier = "team".to_owned();
        q_save_org(&mut c, &t).unwrap();
        run(&mut c, &ev("dsync.user.created", dir_user("directory_user_1", "dana@acme.com", "active")));
        assert!(member_by_email(&mut c, "dana@acme.com").is_none());
    }

    #[test]
    fn provisioning_never_binds_an_existing_non_member_account() {
        let mut c = db();
        bound_tenant(&mut c);
        add_user(&mut c, "personal", "dana@acme.com");
        let before = crate::db::models::reins_org_event::q_events(&mut c, "o1", None, 100).unwrap().len();
        run(&mut c, &ev("dsync.user.created", dir_user("directory_user_1", "dana@acme.com", "active")));
        assert!(reins_org::q_membership(&mut c, "o1", "personal").unwrap().is_none());
        // Nothing the org can read mentions it.
        assert_eq!(crate::db::models::reins_org_event::q_events(&mut c, "o1", None, 100).unwrap().len(), before);
        // Its deletion from the directory touches nothing either.
        assert!(
            run(&mut c, &ev("dsync.user.deleted", dir_user("directory_user_1", "dana@acme.com", "inactive")))
                .is_empty()
        );
        let enabled: bool =
            users::table.filter(users::uuid.eq("personal")).select(users::enabled).first(&mut c).unwrap();
        assert!(enabled);
    }

    #[test]
    fn new_users_become_members_waiting_for_their_first_sign_in() {
        let mut c = db();
        bound_tenant(&mut c);
        run(&mut c, &ev("dsync.user.created", dir_user("directory_user_1", "lee@acme.com", "active")));
        let m = member_by_email(&mut c, "lee@acme.com").unwrap();
        assert_eq!((m.status, m.external_id.as_deref()), (ACTIVE_STATUS, Some("directory_user_1")));
        let invited: i64 = invitations::table.count().get_result(&mut c).unwrap();
        assert_eq!(invited, 1);
        // A member who joined first is linked, not duplicated.
        add_user(&mut c, "creator", "owner@acme.com");
        add_member(&mut c, "o1", "m_owner", "creator", ACTIVE_STATUS);
        run(&mut c, &ev("dsync.user.created", dir_user("directory_user_2", "owner@acme.com", "active")));
        let m = reins_org::q_membership(&mut c, "o1", "creator").unwrap().unwrap();
        assert_eq!(m.external_id.as_deref(), Some("directory_user_2"));
    }

    #[test]
    fn groups_and_memberships_follow_the_directory() {
        let mut c = db();
        bound_tenant(&mut c);
        run(&mut c, &ev("dsync.user.created", dir_user("directory_user_1", "lee@acme.com", "active")));
        let group = json!({"object": "directory_group", "id": "directory_group_1", "name": "Security",
                           "directory_id": "directory_ACME", "organization_id": "org_ACME"});
        run(&mut c, &ev("dsync.group.created", group.clone()));
        let membership = json!({"directory_id": "directory_ACME",
                                "user": dir_user("directory_user_1", "lee@acme.com", "active"), "group": group});
        run(&mut c, &ev("dsync.group.user_added", membership.clone()));
        run(&mut c, &ev("dsync.group.user_added", membership.clone()));
        let g = group_by_external(&mut c, "o1", "directory_group_1").unwrap().unwrap();
        let m = member_by_email(&mut c, "lee@acme.com").unwrap();
        assert_eq!(reins_org::q_group_members(&mut c, &g).unwrap(), [m.uuid.to_string()]);
        // Roles given to the group reach its members, and go with the group.
        grant(&mut c, "o1", SUBJECT_GROUP, &g, reins_proto::org::Role::Approver);
        assert_eq!(reins_org::q_roles_of(&mut c, "o1", m.uuid.as_ref()).unwrap(), [reins_proto::org::Role::Approver]);
        run(&mut c, &ev("dsync.group.user_removed", membership));
        assert!(reins_org::q_group_members(&mut c, &g).unwrap().is_empty());
        run(&mut c, &ev("dsync.group.deleted", group));
        assert!(group_by_external(&mut c, "o1", "directory_group_1").unwrap().is_none());
        assert!(reins_org::q_subject_roles(&mut c, "o1", SUBJECT_GROUP, &g).unwrap().is_empty());
    }

    #[test]
    fn suspend_and_delete_deprovision_and_reactivation_restores() {
        let mut c = db();
        bound_tenant(&mut c);
        run(&mut c, &ev("dsync.user.created", dir_user("directory_user_1", "lee@acme.com", "active")));
        let removed = run(&mut c, &ev("dsync.user.updated", dir_user("directory_user_1", "lee@acme.com", "inactive")));
        assert_eq!(removed.len(), 1);
        assert!(member_by_email(&mut c, "lee@acme.com").unwrap().status < 0);
        run(&mut c, &ev("dsync.user.updated", dir_user("directory_user_1", "lee@acme.com", "active")));
        let m = member_by_email(&mut c, "lee@acme.com").unwrap();
        assert!(m.status >= ACTIVE_STATUS);
        let removed = run(&mut c, &ev("dsync.user.deleted", dir_user("directory_user_1", "lee@acme.com", "inactive")));
        assert_eq!(removed.len(), 1);
        let user = m.user_uuid.to_string();
        assert_eq!(
            reins_org::q_member_state(&mut c, "o1", &user).unwrap().unwrap().state,
            state::DEPROVISIONED_BY_DIRECTORY
        );
        // Deleted is final: "active" again changes nothing.
        run(&mut c, &ev("dsync.user.updated", dir_user("directory_user_1", "lee@acme.com", "active")));
        assert_eq!(
            reins_org::q_member_state(&mut c, "o1", &user).unwrap().unwrap().state,
            state::DEPROVISIONED_BY_DIRECTORY
        );
    }

    #[test]
    fn the_directory_cannot_undo_an_admin_removal() {
        let mut c = db();
        bound_tenant(&mut c);
        run(&mut c, &ev("dsync.user.created", dir_user("directory_user_1", "lee@acme.com", "active")));
        let user = member_by_email(&mut c, "lee@acme.com").unwrap().user_uuid.to_string();
        deprovision::q_deprovision(&mut c, "o1", &user, Reason::AdminRemoval, Some("admin"), 11).unwrap().unwrap();
        run(&mut c, &ev("dsync.user.updated", dir_user("directory_user_1", "lee@acme.com", "active")));
        assert_eq!(reins_org::q_member_state(&mut c, "o1", &user).unwrap().unwrap().state, state::REMOVED_BY_ADMIN);
        // The link is gone, and the same address can't be linked again while the removal stands.
        assert!(membership_by_external(&mut c, "o1", "directory_user_1").unwrap().is_none());
        assert_eq!(reins_org::q_member_state(&mut c, "o1", &user).unwrap().unwrap().state, state::REMOVED_BY_ADMIN);
    }

    #[test]
    fn suspension_applies_even_when_the_address_left_the_domain() {
        let mut c = db();
        bound_tenant(&mut c);
        run(&mut c, &ev("dsync.user.created", dir_user("directory_user_1", "lee@acme.com", "active")));
        let user = member_by_email(&mut c, "lee@acme.com").unwrap().user_uuid.to_string();
        let removed =
            run(&mut c, &ev("dsync.user.updated", dir_user("directory_user_1", "lee@elsewhere.io", "inactive")));
        assert_eq!(removed.len(), 1);
        assert_eq!(
            reins_org::q_member_state(&mut c, "o1", &user).unwrap().unwrap().state,
            state::SUSPENDED_BY_DIRECTORY
        );
    }

    #[test]
    fn another_orgs_member_is_never_linked() {
        let mut c = db();
        bound_tenant(&mut c);
        add_org(&mut c, "o2");
        add_member(&mut c, "o2", "n1", "v1", ACTIVE_STATUS);
        diesel::update(users::table.filter(users::uuid.eq("v1")))
            .set(users::email.eq("vee@acme.com"))
            .execute(&mut c)
            .unwrap();
        run(&mut c, &ev("dsync.user.created", dir_user("directory_user_7", "vee@acme.com", "active")));
        assert!(reins_org::q_membership(&mut c, "o1", "v1").unwrap().is_none());
        assert_eq!(reins_org::q_member_state(&mut c, "o2", "v1").unwrap().unwrap().state, state::ACTIVE);
    }

    #[test]
    fn a_listing_backfills_what_the_events_missed_and_removes_what_is_gone() {
        let mut c = db();
        bound_tenant(&mut c);
        // Lee was provisioned by an event; then the directory was bound to nothing for a while.
        run(&mut c, &ev("dsync.user.created", dir_user("directory_user_lee", "lee@acme.com", "active")));
        let tenant = reins_org::q_org(&mut c, "o1").unwrap().unwrap();
        let mut kim = dir_user("directory_user_kim", "kim@acme.com", "active");
        kim["groups"] = json!([{"id": "directory_group_sec", "name": "Security"}]);
        let groups = [json!({"id": "directory_group_sec", "name": "Security"})];
        let removed = q_backfill(&mut c, &tenant, &[kim], &groups, 20).unwrap();
        // Kim and her group arrive; Lee, gone from the directory, is deprovisioned.
        let kim_m = member_by_email(&mut c, "kim@acme.com").unwrap();
        let g = group_by_external(&mut c, "o1", "directory_group_sec").unwrap().unwrap();
        assert_eq!(reins_org::q_group_members(&mut c, &g).unwrap(), [kim_m.uuid.to_string()]);
        assert_eq!(removed.len(), 1);
        let lee = member_by_email(&mut c, "lee@acme.com").unwrap().user_uuid.to_string();
        assert_eq!(
            reins_org::q_member_state(&mut c, "o1", &lee).unwrap().unwrap().state,
            state::DEPROVISIONED_BY_DIRECTORY
        );
        // A member who joined before the directory (no directory id) is never removed by a listing.
        add_user(&mut c, "creator", "owner@acme.com");
        add_member(&mut c, "o1", "m_owner", "creator", ACTIVE_STATUS);
        q_backfill(&mut c, &tenant, &[], &[], 30).unwrap();
        assert!(reins_org::q_member_state(&mut c, "o1", "creator").unwrap().unwrap().is_active());
    }

    #[test]
    fn the_last_owner_suspended_and_restored_by_the_directory() {
        let mut c = db();
        bound_tenant(&mut c);
        run(&mut c, &ev("dsync.user.created", dir_user("directory_user_1", "lee@acme.com", "active")));
        let m = member_by_email(&mut c, "lee@acme.com").unwrap();
        grant(&mut c, "o1", reins_org::SUBJECT_MEMBER, m.uuid.as_ref(), reins_proto::org::Role::OrgOwner);
        run(&mut c, &ev("dsync.user.updated", dir_user("directory_user_1", "lee@acme.com", "inactive")));
        // Kept as owner, cut off, flagged.
        assert!(super::super::invariants::recovery_mode(&mut c, "o1").unwrap());
        assert!(super::super::invariants::owner_recovery_shown(&mut c, "o1").unwrap());
        run(&mut c, &ev("dsync.user.updated", dir_user("directory_user_1", "lee@acme.com", "active")));
        assert!(!super::super::invariants::recovery_mode(&mut c, "o1").unwrap());
        assert!(!super::super::invariants::owner_recovery_shown(&mut c, "o1").unwrap());
        // The account's global flag was never touched on the way.
        let enabled: bool =
            users::table.filter(users::uuid.eq(m.user_uuid.to_string())).select(users::enabled).first(&mut c).unwrap();
        assert!(enabled);
    }
}
