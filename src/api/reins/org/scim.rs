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
use serde::{Deserialize, Serialize};
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
            reins_workos::{q_delete_setting, q_get_setting, q_set_setting},
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
        /// The directory's `manager_email` custom attribute (O3), lowercase.
        manager_email: Option<String>,
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
                manager_email: d
                    .get("custom_attributes")
                    .and_then(|a| str_at(a, "manager_email"))
                    .map(|e| e.trim().to_lowercase()),
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
            manager_email,
            ..
        } => {
            let removals = upsert_user(c, org, user_id, email, name.as_deref(), *active, now)?;
            if *active && let Some(m) = membership_by_external(c, org, user_id)? {
                directory_manager(c, org, m.user_uuid.as_ref(), manager_email.as_deref(), now)?;
            }
            Ok(removals)
        }
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
            q_directory_changed(c, org, t.directory_id.as_deref(), None, now)?;
            t.directory_id = None;
            t.updated_at = now;
            reins_org::q_save_org(c, &t)?;
            event(c, org, "directory.unbound", None, None, &json!({}), now)?;
            Ok(Vec::new())
        }
        DirAction::Ignore => Ok(Vec::new()),
    }
}

/// Sets a member's manager from the directory: only an active member of the same tenant (never someone outside it,
/// never themselves). Anything else clears the link, so a stale one can't route a review. On a tier with a directory
/// the directory's word replaces any link, an admin-set one included (admin links are for tiers without one).
fn directory_manager(
    c: &mut DbConnInner,
    org: &str,
    user: &str,
    manager_email: Option<&str>,
    now: i64,
) -> Result<(), OrgError> {
    use crate::db::models::reins_org_routing::{self as routing, manager_source};
    let manager = match manager_email {
        Some(e) => {
            let found: Option<String> =
                users::table.filter(users::email.eq(e)).select(users::uuid).first(c).optional()?;
            match found {
                Some(m) if m != user && super::member_of(c, org, &m)?.is_some() => Some(m),
                _ => None,
            }
        }
        None => None,
    };
    let current = routing::q_person(c, org, user)?;
    let same = current.as_ref().is_some_and(|p| {
        p.manager_user.as_deref() == manager.as_deref()
            && p.manager_source.as_deref() == manager.as_ref().map(|_| manager_source::DIRECTORY)
    });
    if same || (current.is_none() && manager.is_none()) {
        return Ok(());
    }
    routing::q_edit_person(c, org, user, now, |p| {
        p.manager_source = manager.as_ref().map(|_| manager_source::DIRECTORY.to_owned());
        p.manager_user = manager;
    })?;
    Ok(())
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
        if !crate::util::is_valid_email(email) {
            event(c, org, "scim.refused", None, Some(user_id), &json!({"why": "invalid_email"}), now)?;
            return Ok(Vec::new());
        }
        let mut stub = User::new(email, name.map(str::to_owned));
        stub.verified_at = None;
        stub.ready_to_write().map_err(|e| OrgError::Internal(format!("{e:?}")))?;
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
const BACKFILL_EVERY: Duration = Duration::from_mins(30);

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
        let (data, next) = parse_list_page(&page)?;
        out.extend(data);
        after = next;
        if after.is_none() {
            return Ok(out);
        }
    }
    Err("the directory listing did not end".to_owned())
}

/// One page of a WorkOS list: its items and the cursor of the next page. A page without a `data` array or without
/// `list_metadata` is an error, never an empty page: a backfill reads absence as removal.
pub fn parse_list_page(page: &Value) -> Result<(Vec<Value>, Option<String>), String> {
    let data = page.get("data").and_then(Value::as_array).ok_or("a WorkOS list without data")?;
    let meta = page.get("list_metadata").and_then(Value::as_object).ok_or("a WorkOS list without list_metadata")?;
    let after = match meta.get("after") {
        None | Some(Value::Null) => None,
        Some(Value::String(a)) if !a.is_empty() => Some(a.clone()),
        Some(_) => return Err("a WorkOS list with an unreadable cursor".to_owned()),
    };
    Ok((data.clone(), after))
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

/// Most members one backfill removes for being absent from the listing without an admin confirming: 5, or a tenth
/// of the linked members if that is more.
#[must_use]
pub fn removal_limit(linked: usize) -> usize {
    (linked / 10).max(5)
}

fn held_key(org: &str) -> String {
    format!("dsync.held.{org}")
}

fn confirmed_key(org: &str) -> String {
    format!("dsync.confirmed.{org}")
}

/// Members (their directory ids) a backfill held back until an admin confirms ([`reins_proto::org::AdminAction::ConfirmDirectoryRemovals`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Held {
    pub external_ids: Vec<String>,
    pub digest: String,
}

impl Held {
    fn of(mut ids: Vec<String>) -> Self {
        ids.sort();
        ids.dedup();
        let digest = crate::crypto::sha256_hex(ids.join("\n").as_bytes());
        Self {
            external_ids: ids,
            digest,
        }
    }
}

/// What the last backfill held back in `org`, if anything.
pub fn q_held(c: &mut DbConnInner, org: &str) -> Result<Option<Held>, OrgError> {
    Ok(q_get_setting(c, &held_key(org))?.and_then(|v| serde_json::from_str(&v).ok()))
}

/// The memberships a held removal would end (for the capability confirming it takes).
pub fn q_held_memberships(c: &mut DbConnInner, org: &str) -> Result<Vec<String>, OrgError> {
    let Some(held) = q_held(c, org)? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for id in &held.external_ids {
        if let Some(m) = membership_by_external(c, org, id)? {
            out.push(m.uuid.to_string());
        }
    }
    Ok(out)
}

/// An admin confirmed the held removals `digest` names: the next backfill removes those members if the directory still
/// doesn't list them. Refused when what's held changed since the admin looked.
pub fn q_confirm_held(c: &mut DbConnInner, org: &str, digest: &str, check_only: bool) -> Result<usize, OrgError> {
    let Some(held) = q_held(c, org)?.filter(|h| crate::crypto::ct_eq(&h.digest, digest)) else {
        return Err(OrgError::Invalid("The directory changed since. Look again."));
    };
    if !check_only {
        let ids = serde_json::to_string(&held.external_ids).map_err(|e| OrgError::Internal(e.to_string()))?;
        q_set_setting(c, &confirmed_key(org), &ids)?;
    }
    Ok(held.external_ids.len())
}

/// The tenant's directory changed (another one bound, or none): members and groups the old one linked keep their
/// membership and roles, but no longer carry its ids, so the new directory links them by address and never removes
/// them for being absent from it. What a backfill of the old one held is dropped.
pub fn q_directory_changed(
    c: &mut DbConnInner,
    org: &str,
    old: Option<&str>,
    new: Option<&str>,
    now: i64,
) -> Result<(), OrgError> {
    if old.is_none() || old == new {
        return Ok(());
    }
    let members = diesel::update(
        users_organizations::table
            .filter(users_organizations::org_uuid.eq(org))
            .filter(users_organizations::external_id.is_not_null()),
    )
    .set(users_organizations::external_id.eq(None::<String>))
    .execute(c)?;
    let groups_unlinked = diesel::update(
        groups::table.filter(groups::organizations_uuid.eq(org)).filter(groups::external_id.is_not_null()),
    )
    .set(groups::external_id.eq(None::<String>))
    .execute(c)?;
    q_delete_setting(c, &held_key(org))?;
    q_delete_setting(c, &confirmed_key(org))?;
    event(c, org, "directory.unlinked", None, None, &json!({"members": members, "groups": groups_unlinked}), now)?;
    Ok(())
}

/// Applies a complete listing of `tenant`'s directory, in the caller's transaction: its groups, its users and their
/// group memberships, as the events would have. Then, from the listing being complete:
///
/// - directory groups it no longer lists are deleted, and linked members leave the directory groups it no longer
///   lists them in;
/// - linked members it no longer lists are deprovisioned (the directory deleted them while nothing listened), but
///   never on an empty listing while linked members exist, and never more than [`removal_limit`] at once unless an
///   admin confirmed exactly those ([`q_confirm_held`]); otherwise they're held, and the console shows them.
pub fn q_backfill(
    c: &mut DbConnInner,
    tenant: &ReinsOrg,
    users: &[Value],
    groups: &[Value],
    now: i64,
) -> Result<Vec<Removal>, OrgError> {
    use std::collections::HashSet;
    let (Some(directory), Some(workos)) = (tenant.directory_id.as_deref(), tenant.workos_org_id.as_deref()) else {
        return Ok(Vec::new());
    };
    let event_of = |kind: &str, data: &Value| {
        let mut data = data.clone();
        data["directory_id"] = json!(directory);
        data["organization_id"] = json!(workos);
        Event {
            id: String::new(),
            event: kind.to_owned(),
            data,
        }
    };
    let org = tenant.org_uuid.as_str();
    let mut removals = Vec::new();
    let mut listed_groups = HashSet::new();
    for g in groups {
        if let Some(id) = g.get("id").and_then(Value::as_str) {
            listed_groups.insert(id.to_owned());
        }
        removals.extend(apply(c, &plan(&event_of("dsync.group.updated", g)), now)?);
    }
    let mut listed = HashSet::new();
    let mut pairs = HashSet::new();
    for u in users {
        let Some(user_id) = u.get("id").and_then(Value::as_str) else {
            continue;
        };
        listed.insert(user_id.to_owned());
        removals.extend(apply(c, &plan(&event_of("dsync.user.updated", u)), now)?);
        for g in u.get("groups").and_then(Value::as_array).cloned().unwrap_or_default() {
            if let Some(group_id) = g.get("id").and_then(Value::as_str) {
                pairs.insert((user_id.to_owned(), group_id.to_owned()));
                let added =
                    event_of("dsync.group.user_added", &json!({"user": {"id": user_id}, "group": {"id": group_id}}));
                removals.extend(apply(c, &plan(&added), now)?);
            }
        }
    }

    // Directory groups the listing no longer has. An empty group listing deletes nothing (deletions also come as
    // events).
    let directory_groups: Vec<(String, String)> = groups::table
        .filter(groups::organizations_uuid.eq(org))
        .filter(groups::external_id.is_not_null())
        .select((groups::uuid, groups::external_id.assume_not_null()))
        .load(c)?;
    if !listed_groups.is_empty() {
        for (uuid, external) in &directory_groups {
            if !listed_groups.contains(external) {
                diesel::delete(groups_users::table.filter(groups_users::groups_uuid.eq(uuid))).execute(c)?;
                reins_org::q_revoke_all(c, org, SUBJECT_GROUP, uuid)?;
                diesel::delete(groups::table.filter(groups::uuid.eq(uuid))).execute(c)?;
                event(c, org, "group.deprovisioned", None, Some(uuid), &json!({"backfill": true}), now)?;
            }
        }
    }

    // Linked members, and which of them the listing left out.
    let mut linked = Vec::new();
    for m in reins_org::q_members(c, org)? {
        let Some(external) = m.external_id.clone() else {
            continue;
        };
        let user = m.user_uuid.to_string();
        if reins_org::q_member_state(c, org, &user)?.is_some_and(|s| s.is_active()) {
            linked.push((m.uuid.to_string(), user, external));
        }
    }

    // Directory-group memberships of listed members that the listing no longer pairs.
    for (membership, _, external) in linked.iter().filter(|(_, _, x)| listed.contains(x)) {
        for (group, group_external) in &directory_groups {
            if pairs.contains(&(external.clone(), group_external.clone())) {
                continue;
            }
            let gone = diesel::delete(
                groups_users::table
                    .filter(groups_users::groups_uuid.eq(group))
                    .filter(groups_users::users_organizations_uuid.eq(membership)),
            )
            .execute(c)?;
            if gone > 0 {
                event(c, org, "group.member_removed", None, Some(group), &json!({"member": membership}), now)?;
            }
        }
    }

    let absent: Vec<&(String, String, String)> = linked.iter().filter(|(_, _, x)| !listed.contains(x)).collect();
    if absent.is_empty() {
        q_delete_setting(c, &held_key(org))?;
        q_delete_setting(c, &confirmed_key(org))?;
        return Ok(removals);
    }
    if listed.is_empty() {
        // A listing without a single user, while members are linked: a broken listing, not an empty company.
        event(
            c,
            org,
            "directory.removals_held",
            None,
            None,
            &json!({"count": absent.len(), "why": "empty_listing"}),
            now,
        )?;
        return Ok(removals);
    }
    let confirmed: HashSet<String> = q_get_setting(c, &confirmed_key(org))?
        .and_then(|v| serde_json::from_str::<Vec<String>>(&v).ok())
        .unwrap_or_default()
        .into_iter()
        .collect();
    let all_confirmed = absent.iter().all(|(_, _, x)| confirmed.contains(x));
    if absent.len() > removal_limit(linked.len()) && !all_confirmed {
        let held = Held::of(absent.iter().map(|(_, _, x)| x.clone()).collect());
        let changed = q_held(c, org)?.is_none_or(|h| h.digest != held.digest);
        q_set_setting(
            c,
            &held_key(org),
            &serde_json::to_string(&held).map_err(|e| OrgError::Internal(e.to_string()))?,
        )?;
        if changed {
            event(
                c,
                org,
                "directory.removals_held",
                None,
                None,
                &json!({"count": absent.len(), "why": "too_many"}),
                now,
            )?;
        }
        return Ok(removals);
    }
    for (_, user, _) in absent {
        removals.extend(deprovision::q_deprovision(c, org, user, Reason::DirectoryDelete, None, now)?);
    }
    q_delete_setting(c, &held_key(org))?;
    q_delete_setting(c, &confirmed_key(org))?;
    Ok(removals)
}

/// Lists and applies every bound directory. A listing that fails changes nothing for that tenant.
pub async fn backfill_all(
    listing: &dyn DirectoryListing,
    conn: &DbConn,
    pool: Option<&DbPool>,
) -> Result<usize, String> {
    let tenants = conn.run(reins_org::q_bound_directories).await.map_err(|e| e.to_string())?;
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
                manager_email: None,
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
    fn a_list_page_without_data_or_metadata_is_an_error() {
        let ok = json!({"data": [{"id": "u1"}], "list_metadata": {"after": "u1", "before": null}});
        assert_eq!(parse_list_page(&ok).unwrap(), (vec![json!({"id": "u1"})], Some("u1".to_owned())));
        let last = json!({"data": [], "list_metadata": {"after": null}});
        assert_eq!(parse_list_page(&last).unwrap(), (vec![], None));
        for bad in [
            json!({"list_metadata": {"after": null}}),
            json!({"data": null, "list_metadata": {"after": null}}),
            json!({"data": {"id": "u1"}, "list_metadata": {}}),
            json!({"data": []}),
            json!({"data": [], "list_metadata": null}),
            json!({"data": [], "list_metadata": {"after": 7}}),
            json!({"error": "rate limited"}),
        ] {
            assert!(parse_list_page(&bad).is_err(), "{bad}");
        }
    }

    /// `n` members the directory linked, as a listing returns them.
    fn linked(c: &mut DbConnInner, n: usize) -> Vec<Value> {
        let users: Vec<Value> =
            (0..n).map(|i| dir_user(&format!("directory_user_{i}"), &format!("p{i}@acme.com"), "active")).collect();
        for u in &users {
            run(c, &ev("dsync.user.created", u.clone()));
        }
        users
    }

    fn active_count(c: &mut DbConnInner) -> usize {
        reins_org::q_members(c, "o1")
            .unwrap()
            .into_iter()
            .filter(|m| {
                reins_org::q_member_state(c, "o1", m.user_uuid.as_ref()).unwrap().is_some_and(|s| s.is_active())
            })
            .count()
    }

    #[test]
    fn an_empty_listing_removes_nobody() {
        let mut c = db();
        bound_tenant(&mut c);
        linked(&mut c, 3);
        let tenant = reins_org::q_org(&mut c, "o1").unwrap().unwrap();
        assert!(q_backfill(&mut c, &tenant, &[], &[], 20).unwrap().is_empty());
        assert_eq!(active_count(&mut c), 3);
    }

    #[test]
    fn a_mass_removal_waits_for_an_admin() {
        let mut c = db();
        bound_tenant(&mut c);
        let users = linked(&mut c, 20);
        add_user(&mut c, "creator", "owner@acme.com");
        add_member(&mut c, "o1", "m_owner", "creator", ACTIVE_STATUS);
        grant(&mut c, "o1", reins_org::SUBJECT_MEMBER, "m_owner", reins_proto::org::Role::OrgOwner);
        let tenant = reins_org::q_org(&mut c, "o1").unwrap().unwrap();
        // Up to the limit (5 of 20) goes through.
        assert_eq!(removal_limit(20), 5);
        assert_eq!(q_backfill(&mut c, &tenant, &users[5..], &[], 20).unwrap().len(), 5);
        assert!(q_held(&mut c, "o1").unwrap().is_none());
        // Six more of the fifteen left: held, nobody removed, and the console has something to show.
        assert!(q_backfill(&mut c, &tenant, &users[11..], &[], 30).unwrap().is_empty());
        assert_eq!(active_count(&mut c), 16);
        let held = q_held(&mut c, "o1").unwrap().unwrap();
        assert_eq!(held.external_ids.len(), 6);
        // A confirmation of something else (stale) is refused; of exactly what's held, it's recorded.
        let stale = reins_proto::org::AdminAction::ConfirmDirectoryRemovals {
            held: "stale".to_owned(),
        };
        assert!(super::super::admin::propose(&mut c, "o1", "creator", &stale, None, 40).is_err());
        let confirm = reins_proto::org::AdminAction::ConfirmDirectoryRemovals {
            held: held.digest.clone(),
        };
        super::super::admin::propose(&mut c, "o1", "creator", &confirm, None, 40).unwrap();
        // The next backfill removes them, and only them.
        assert_eq!(q_backfill(&mut c, &tenant, &users[11..], &[], 50).unwrap().len(), 6);
        assert_eq!(active_count(&mut c), 10);
        assert!(q_held(&mut c, "o1").unwrap().is_none());
        // A confirmation is spent: another mass removal is held again.
        assert!(q_backfill(&mut c, &tenant, &users[19..], &[], 60).unwrap().is_empty());
        assert!(q_held(&mut c, "o1").unwrap().is_some());
    }

    #[test]
    fn a_complete_listing_reconciles_groups() {
        let mut c = db();
        bound_tenant(&mut c);
        let tenant = reins_org::q_org(&mut c, "o1").unwrap().unwrap();
        let sec = json!({"id": "directory_group_sec", "name": "Security"});
        let ops = json!({"id": "directory_group_ops", "name": "Ops"});
        let mut kim = dir_user("directory_user_kim", "kim@acme.com", "active");
        kim["groups"] = json!([sec.clone(), ops.clone()]);
        q_backfill(&mut c, &tenant, std::slice::from_ref(&kim), &[sec.clone(), ops.clone()], 10).unwrap();
        let g_sec = group_by_external(&mut c, "o1", "directory_group_sec").unwrap().unwrap();
        let g_ops = group_by_external(&mut c, "o1", "directory_group_ops").unwrap().unwrap();
        let kim_m = member_by_email(&mut c, "kim@acme.com").unwrap().uuid.to_string();
        assert_eq!(reins_org::q_group_members(&mut c, &g_sec).unwrap(), [kim_m.clone()]);
        // A group made in Reins (no directory id) is never touched by a listing.
        add_group(&mut c, "o1", "g_local", &[&kim_m]);
        grant(&mut c, "o1", SUBJECT_GROUP, &g_ops, reins_proto::org::Role::Approver);
        // The directory dropped Ops, and Kim from Security (events missed).
        kim["groups"] = json!([]);
        q_backfill(&mut c, &tenant, std::slice::from_ref(&kim), std::slice::from_ref(&sec), 20).unwrap();
        assert!(group_by_external(&mut c, "o1", "directory_group_ops").unwrap().is_none());
        assert!(reins_org::q_subject_roles(&mut c, "o1", SUBJECT_GROUP, &g_ops).unwrap().is_empty());
        assert!(reins_org::q_group_members(&mut c, &g_sec).unwrap().is_empty());
        assert_eq!(reins_org::q_group_members(&mut c, "g_local").unwrap(), [kim_m]);
        // An empty group listing deletes no group.
        q_backfill(&mut c, &tenant, std::slice::from_ref(&kim), &[], 30).unwrap();
        assert!(group_by_external(&mut c, "o1", "directory_group_sec").unwrap().is_some());
    }

    #[test]
    fn a_new_directory_relinks_members_and_never_removes_them() {
        let mut c = db();
        bound_tenant(&mut c);
        linked(&mut c, 3);
        run(
            &mut c,
            &ev(
                "dsync.group.created",
                json!({"id": "directory_group_sec", "name": "Security",
            "directory_id": "directory_ACME", "organization_id": "org_ACME"}),
            ),
        );
        let g = group_by_external(&mut c, "o1", "directory_group_sec").unwrap().unwrap();
        // The tenant moves to another directory.
        let mut t = reins_org::q_org(&mut c, "o1").unwrap().unwrap();
        q_directory_changed(&mut c, "o1", t.directory_id.as_deref(), Some("directory_NEW"), 40).unwrap();
        t.directory_id = Some("directory_NEW".to_owned());
        q_save_org(&mut c, &t).unwrap();
        // The new directory lists only one of them, under a new id: nobody is removed for being absent from it.
        let mut p0 = dir_user("directory_user_new0", "p0@acme.com", "active");
        p0["directory_id"] = json!("directory_NEW");
        assert!(q_backfill(&mut c, &t, &[p0], &[], 50).unwrap().is_empty());
        assert_eq!(active_count(&mut c), 3);
        // The one it lists is linked to it again; the old group stays, a plain group now.
        assert_eq!(member_by_email(&mut c, "p0@acme.com").unwrap().external_id.as_deref(), Some("directory_user_new0"));
        assert!(member_by_email(&mut c, "p1@acme.com").unwrap().external_id.is_none());
        assert_eq!(reins_org::q_groups(&mut c, "o1").unwrap().into_iter().filter(|(id, _, _)| *id == g).count(), 1);
    }

    #[test]
    fn a_directory_user_with_an_invalid_address_is_refused() {
        let mut c = db();
        bound_tenant(&mut c);
        run(&mut c, &ev("dsync.user.created", dir_user("directory_user_x", "a b@acme.com", "active")));
        assert!(member_by_email(&mut c, "a b@acme.com").is_none());
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

    #[test]
    fn managers_come_from_the_directory_within_the_tenant() {
        use crate::db::models::reins_org_routing::q_person;
        let mut c = db();
        bound_tenant(&mut c);
        let with_manager = |id: &str, email: &str, manager: &str| {
            let mut u = dir_user(id, email, "active");
            u["custom_attributes"] = json!({"manager_email": manager});
            u
        };
        run(&mut c, &ev("dsync.user.created", dir_user("directory_user_mia", "mia@acme.com", "active")));
        run(&mut c, &ev("dsync.user.created", with_manager("directory_user_lee", "lee@acme.com", "Mia@Acme.com")));
        let (lee, mia) =
            (member_by_email(&mut c, "lee@acme.com").unwrap(), member_by_email(&mut c, "mia@acme.com").unwrap());
        let person = q_person(&mut c, "o1", lee.user_uuid.as_ref()).unwrap().unwrap();
        assert_eq!(person.manager_user.as_deref(), Some(mia.user_uuid.to_string().as_str()));
        assert_eq!(person.manager_source.as_deref(), Some("directory"));
        // A manager outside the tenant is ignored, and clears the directory's earlier word.
        add_user(&mut c, "boss", "boss@other.com");
        run(&mut c, &ev("dsync.user.updated", with_manager("directory_user_lee", "lee@acme.com", "boss@other.com")));
        assert_eq!(q_person(&mut c, "o1", lee.user_uuid.as_ref()).unwrap().unwrap().manager_user, None);
        // Nor is anyone their own manager.
        run(&mut c, &ev("dsync.user.updated", with_manager("directory_user_lee", "lee@acme.com", "lee@acme.com")));
        assert_eq!(q_person(&mut c, "o1", lee.user_uuid.as_ref()).unwrap().unwrap().manager_user, None);
    }
    #[test]
    fn a_directory_manager_change_replaces_an_admin_set_manager() {
        use crate::db::models::reins_org_routing::{manager_source, q_edit_person, q_person};
        let mut c = db();
        bound_tenant(&mut c);
        let with_manager = |id: &str, email: &str, manager: &str| {
            let mut u = dir_user(id, email, "active");
            u["custom_attributes"] = json!({"manager_email": manager});
            u
        };
        run(&mut c, &ev("dsync.user.created", dir_user("directory_user_mia", "mia@acme.com", "active")));
        run(&mut c, &ev("dsync.user.created", dir_user("directory_user_old", "old@acme.com", "active")));
        run(&mut c, &ev("dsync.user.created", dir_user("directory_user_lee", "lee@acme.com", "active")));
        let lee = member_by_email(&mut c, "lee@acme.com").unwrap().user_uuid.to_string();
        let old = member_by_email(&mut c, "old@acme.com").unwrap().user_uuid.to_string();
        let mia = member_by_email(&mut c, "mia@acme.com").unwrap().user_uuid.to_string();
        // An admin link from before (a Team-era change): on a tier with a directory it doesn't count, and the
        // directory's word replaces it.
        q_edit_person(&mut c, "o1", &lee, 5, |p| {
            p.manager_user = Some(old.clone());
            p.manager_source = Some(manager_source::ADMIN.to_owned());
        })
        .unwrap();
        assert!(!crate::api::reins::org::routing::manager_links(&mut c, "o1").unwrap().contains_key(&lee));
        run(&mut c, &ev("dsync.user.updated", with_manager("directory_user_lee", "lee@acme.com", "mia@acme.com")));
        let p = q_person(&mut c, "o1", &lee).unwrap().unwrap();
        assert_eq!((p.manager_user.as_deref(), p.manager_source.as_deref()), (Some(mia.as_str()), Some("directory")));
        // And the directory saying nobody clears it.
        run(&mut c, &ev("dsync.user.updated", dir_user("directory_user_lee", "lee@acme.com", "active")));
        assert_eq!(q_person(&mut c, "o1", &lee).unwrap().unwrap().manager_user, None);
    }
}
