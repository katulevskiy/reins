//! Approval routing (`docs/design/decisions-v2/05-organizations.md` §6): who an org review obligation names, resolved
//! from **verified sources only** (05 §7.1) at submit, and stored on the request and in the countersigned decision.
//!
//! - **Groups, roles, named members** come from the member's standing (the issuer-signed roster). Nothing is stored for
//!   them at the first step: they're checked live, by the decision service and by executors.
//! - **The manager** comes from the directory (SCIM `manager_email`), or from a two-person admin change on the Team
//!   tier, which has no directory. Never from the agent.
//! - **A resource's owner** comes from the policy's verified resource tags.
//! - **Code owners** come from the repository's default branch through the org's read-only GitHub App
//!   ([`codeowners`]); **on-call** from the provider's API ([`oncall`]). Both are fetched before the decision
//!   service's transaction ([`prefetch`]) and handed in as [`Dynamic`]; a source that didn't answer resolves to nobody
//!   and the obligation escalates.
//!
//! Whoever a resolution names must be an active member with a device that may decide (06 §6) or a confirmed web
//! passkey, and is never the requester (05 §6: no self-approval, whatever set they fall in). An obligation with fewer
//! eligible people than its quorum escalates at once ([`escalation`]); escalation adds reviewers and never removes any.
//! Every resolution is audited with its source and evidence (`review.resolved`).

pub mod codeowners;
pub mod escalation;
pub mod oncall;
pub mod sla;

use std::collections::{BTreeMap, BTreeSet};

use reins_policy::org::{
    ManagedOutcome, ManagedRequest, Obligation, OrgPolicy, ResolvedSets, ReviewSpec, Reviewers, analyze_request,
    evaluate_managed,
};
use reins_proto::{managed::Roster, org::Role};
use serde_json::{Value, json};

use super::{OrgError, audit};
use crate::db::{
    DbConnInner, DbPool,
    models::{
        reins_org,
        reins_org_routing::{self, ReinsOrgStep},
    },
};

/// Who a reviewer set resolved to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resolution {
    pub members: BTreeSet<String>,
    /// Short reasons nobody (or not everyone) could be named, for the card and the trace.
    pub unresolved: Vec<String>,
    /// Where they came from: `roster`, `directory`, `admin`, `tags`, `settings`, `codeowners`, `oncall:<provider>`.
    pub source: String,
    /// SHA-256 (hex) of the provider's response, or empty.
    pub evidence: String,
}

impl Resolution {
    #[must_use]
    pub fn from(members: BTreeSet<String>, source: &str) -> Self {
        Self {
            members,
            unresolved: Vec::new(),
            source: source.to_owned(),
            evidence: String::new(),
        }
    }

    #[must_use]
    pub fn nobody(source: &str, why: &str) -> Self {
        Self {
            members: BTreeSet::new(),
            unresolved: vec![why.to_owned()],
            source: source.to_owned(),
            evidence: String::new(),
        }
    }

    fn join(&mut self, other: Self) {
        self.members.extend(other.members);
        self.unresolved.extend(other.unresolved);
        for (mine, theirs) in [(&mut self.source, other.source), (&mut self.evidence, other.evidence)] {
            if !theirs.is_empty() && !mine.split(',').any(|x| x == theirs) {
                if !mine.is_empty() {
                    mine.push(',');
                }
                mine.push_str(&theirs);
            }
        }
    }
}

/// What the dynamic sources said before the decision service's transaction: [`dynamic_key`] → resolution.
pub type Dynamic = BTreeMap<String, Resolution>;

/// The repository and paths a request touches, for code owners.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Touches {
    /// `owner/name`.
    pub repo: Option<String>,
    /// The changed paths, when known (a PR merge's files); `None` takes the default owners.
    pub paths: Option<Vec<String>>,
}

/// The key a dynamic reviewer set's answer is kept under.
#[must_use]
pub fn dynamic_key(r: &Reviewers, touches: &Touches) -> Option<String> {
    match r {
        Reviewers::CodeOwners => {
            let paths = touches
                .paths
                .as_ref()
                .map(|p| reins_proto::canon::hex32(&reins_proto::canon::sha256(p.join("\n").as_bytes())));
            Some(format!("codeowners:{}:{}", touches.repo.as_deref().unwrap_or_default(), paths.unwrap_or_default()))
        }
        Reviewers::OnCall {
            alias,
        } => Some(format!("oncall:{alias}")),
        _ => None,
    }
}

/// The repository a request acts on, when one: a lease's, a GitHub tool's `repo`, or a `github_api_*` path's.
#[must_use]
pub fn repo_of(tool: &str, args: &Value) -> Option<String> {
    if let Some(r) = args.get("repo").and_then(Value::as_str) {
        return Some(r.to_ascii_lowercase());
    }
    if tool.starts_with("github_") {
        let path = args.get("path").and_then(Value::as_str)?;
        let mut parts = path.trim_start_matches('/').split('/');
        if parts.next() == Some("repos") {
            let (o, n) = (parts.next()?, parts.next()?);
            return Some(format!("{o}/{n}").to_ascii_lowercase());
        }
    }
    None
}

/// The pull request a `github_api_write` merges (`PUT /repos/{o}/{r}/pulls/{n}/merge`).
#[must_use]
pub fn merged_pr(tool: &str, args: &Value) -> Option<(String, u64)> {
    let put = args.get("method").and_then(Value::as_str).is_some_and(|m| m.eq_ignore_ascii_case("PUT"));
    if tool != "github_api_write" || !put {
        return None;
    }
    let path = args.get("path").and_then(Value::as_str)?;
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match parts.as_slice() {
        ["repos", o, r, "pulls", n, "merge"] => Some((format!("{o}/{r}").to_ascii_lowercase(), n.parse().ok()?)),
        _ => None,
    }
}

/// One member as routing sees them: their standing and whether they can decide at all.
#[derive(Clone, Debug)]
pub struct Person {
    pub user: String,
    pub email: String,
    pub groups: Vec<String>,
    pub roles: Vec<Role>,
    /// An enrolled device that may decide now, or a confirmed web passkey.
    pub can_decide: bool,
}

/// The org as routing sees it at one moment: its active members, managers and group settings.
pub struct Directory {
    pub people: BTreeMap<String, Person>,
    /// Member → (their manager, an active member; the link's source).
    pub managers: BTreeMap<String, (String, String)>,
    /// Group → (backup group, window).
    pub groups: BTreeMap<String, (Option<String>, Option<i64>)>,
}

impl Directory {
    /// `web`: whether a web approval may decide this request (L0–L2); otherwise a member who has only a web passkey
    /// can't decide it and doesn't count.
    pub fn load(c: &mut DbConnInner, org: &str, now: i64, web: bool) -> Result<Self, OrgError> {
        let policy = super::policy::active(c, org)?.map(|(p, _)| p);
        let roster: Roster = super::roster::build(c, org, policy.as_ref(), now)?;
        let mut people = BTreeMap::new();
        for m in &roster.members {
            let can_decide = m.devices.iter().any(|d| d.may_decide) || (web && !m.passkeys.is_empty());
            people.insert(
                m.user.clone(),
                Person {
                    user: m.user.clone(),
                    email: m.email.to_ascii_lowercase(),
                    groups: m.groups.clone(),
                    roles: m.roles.clone(),
                    can_decide,
                },
            );
        }
        let mut managers = BTreeMap::new();
        for (user, (m, src)) in manager_links(c, org)? {
            if people.contains_key(&m) && people.contains_key(&user) {
                managers.insert(user, (m, src));
            }
        }
        let groups = reins_org_routing::q_group_settings(c, org)?
            .into_iter()
            .map(|g| (g.group_uuid, (g.backup_group, g.window_secs)))
            .collect();
        Ok(Self {
            people,
            managers,
            groups,
        })
    }

    fn in_group(&self, g: &str) -> BTreeSet<String> {
        self.people.values().filter(|p| p.groups.iter().any(|x| x == g)).map(|p| p.user.clone()).collect()
    }

    fn by_email(&self, email: &str) -> Option<&Person> {
        let e = email.trim().to_ascii_lowercase();
        self.people.values().find(|p| p.email == e)
    }

    /// The window of a step naming `r` (its group's setting, a backup's own), else the obligation's.
    #[must_use]
    pub fn window(&self, r: &Reviewers, spec: &ReviewSpec) -> i64 {
        let group = match r {
            Reviewers::BackupOf(g) => self.groups.get(g).and_then(|s| s.0.clone()),
            other => other.first_group().map(str::to_owned),
        };
        group
            .and_then(|g| self.groups.get(&g).and_then(|s| s.1))
            .filter(|w| *w > 0)
            .unwrap_or_else(|| i64::try_from(spec.window()).unwrap_or(i64::MAX / 4))
    }

    /// Who can satisfy `o` now, given its resolution so far.
    #[must_use]
    pub fn eligible(&self, o: &Obligation, requester: &str, resolved: Option<&BTreeSet<String>>) -> BTreeSet<String> {
        self.people
            .values()
            .filter(|p| p.can_decide)
            .filter(|p| o.spec.eligible_resolved(&p.user, &p.groups, &p.roles, requester, resolved))
            .map(|p| p.user.clone())
            .collect()
    }
}

/// Every member's manager link that counts: (manager, source). On a tier with a directory only the directory's;
/// admin-set links count only where there is no directory (05 §7.1: verified sources only).
pub fn manager_links(c: &mut DbConnInner, org: &str) -> Result<BTreeMap<String, (String, String)>, OrgError> {
    let scim = reins_org::q_org(c, org)?.is_some_and(|t| t.tier().scim());
    Ok(reins_org_routing::q_people(c, org)?
        .into_iter()
        .filter_map(|p| match (p.manager_user, p.manager_source) {
            (Some(m), Some(src)) if m != p.user_uuid => Some((p.user_uuid, (m, src))),
            _ => None,
        })
        .filter(|(_, (_, src))| !scim || src == reins_org_routing::manager_source::DIRECTORY)
        .collect())
}

/// What a resolution is about.
pub struct Ctx<'a> {
    pub org: &'a str,
    /// The request's level allows a web approval (L0–L2).
    pub web: bool,
    pub requester: &'a str,
    pub policy: &'a OrgPolicy,
    /// The request's resources (repositories, recipients, hosts), for resource tags.
    pub resources: &'a [String],
    pub touches: &'a Touches,
    pub dynamic: &'a Dynamic,
}

/// Resolves `r` to members. `everyone` includes the members groups and roles name (an escalation step, which must
/// list them); without it only relationships are resolved (a first step: standing covers the rest).
#[must_use]
pub fn resolve(dir: &Directory, ctx: &Ctx<'_>, r: &Reviewers, everyone: bool) -> Resolution {
    let mut out = match r {
        Reviewers::Requester => Resolution::default(),
        Reviewers::Group(g) => {
            if everyone {
                Resolution::from(dir.in_group(g), "roster")
            } else {
                Resolution::from(BTreeSet::new(), "roster")
            }
        }
        Reviewers::Role(role) => {
            let members = if everyone {
                dir.people.values().filter(|p| p.roles.contains(role)).map(|p| p.user.clone()).collect()
            } else {
                BTreeSet::new()
            };
            Resolution::from(members, "roster")
        }
        Reviewers::Members(list) => {
            let members = if everyone {
                list.iter().filter(|m| dir.people.contains_key(*m)).cloned().collect()
            } else {
                BTreeSet::new()
            };
            Resolution::from(members, "roster")
        }
        Reviewers::Manager => match dir.managers.get(ctx.requester) {
            Some((m, src)) => Resolution::from([m.clone()].into(), src),
            None => Resolution::nobody("directory", "no manager on record"),
        },
        Reviewers::BackupOf(g) => match dir.groups.get(g).and_then(|s| s.0.clone()) {
            Some(b) => Resolution::from(dir.in_group(&b), "settings"),
            None => Resolution::nobody("settings", "no backup group"),
        },
        Reviewers::ResourceOwner {
            tag,
        } => match ctx.policy.tag_owner(tag, ctx.resources) {
            Some(owner) if owner.contains('@') => match dir.by_email(owner) {
                Some(p) => Resolution::from([p.user.clone()].into(), "tags"),
                None => Resolution::nobody("tags", "owner isn't a member"),
            },
            Some(group) => Resolution::from(dir.in_group(group), "tags"),
            None => Resolution::nobody("tags", &format!("no {tag} tag")),
        },
        Reviewers::CodeOwners => dynamic_key(r, ctx.touches)
            .and_then(|k| ctx.dynamic.get(&k).cloned())
            .unwrap_or_else(|| Resolution::nobody("codeowners", "code owners unavailable")),
        Reviewers::OnCall {
            alias,
        } => dynamic_key(r, ctx.touches)
            .and_then(|k| ctx.dynamic.get(&k).cloned())
            .unwrap_or_else(|| Resolution::nobody("oncall", &format!("on-call: {alias} unavailable"))),
        Reviewers::AnyOf(list) => {
            let mut all = Resolution::default();
            for x in list {
                all.join(resolve(dir, ctx, x, everyone));
            }
            all
        }
    };
    // Only people who can decide, never the requester.
    out.members.retain(|m| m != ctx.requester && dir.people.get(m).is_some_and(|p| p.can_decide));
    out
}

/// Who a reviewer set is, in short words ("Manager", "Code owners", "On-call: payments").
pub fn label(c: &mut DbConnInner, org: &str, r: &Reviewers) -> Result<String, OrgError> {
    Ok(match r {
        Reviewers::Requester => "The requester".to_owned(),
        Reviewers::Group(g) => group_name(c, org, g)?,
        Reviewers::Role(role) => role.label().to_owned(),
        Reviewers::Members(_) => "Named reviewers".to_owned(),
        Reviewers::Manager => "Manager".to_owned(),
        Reviewers::CodeOwners => "Code owners".to_owned(),
        Reviewers::ResourceOwner {
            tag,
        } => format!("Owner ({tag})"),
        Reviewers::OnCall {
            alias,
        } => format!("On-call: {alias}"),
        Reviewers::BackupOf(g) => format!("Backup of {}", group_name(c, org, g)?),
        Reviewers::AnyOf(list) => {
            let mut parts = Vec::with_capacity(list.len());
            for x in list {
                parts.push(label(c, org, x)?);
            }
            parts.join(" or ")
        }
    })
}

/// A group's name, or "a group".
pub fn group_name(c: &mut DbConnInner, org: &str, group: &str) -> Result<String, OrgError> {
    Ok(reins_org::q_groups(c, org)?
        .into_iter()
        .find(|(id, _, _)| id == group)
        .map_or_else(|| "a group".to_owned(), |g| g.1))
}

/// A request's resolution, as stored.
#[must_use]
pub fn stored(text: Option<&str>) -> ResolvedSets {
    text.and_then(|t| serde_json::from_str(t).ok()).unwrap_or_default()
}

/// What opening a review routed: the resolution, the deadline, and whom to tell.
#[derive(Debug, Default)]
pub struct Routed {
    pub resolved: ResolvedSets,
    pub expires_at: i64,
    pub notify: BTreeSet<String>,
}

/// Routes a new request's obligations (inside the decision service's transaction): resolves each, records step 0,
/// escalates at once while fewer than its quorum can decide, audits every resolution.
pub fn open(
    c: &mut DbConnInner,
    request: &str,
    ctx: &Ctx<'_>,
    obligations: &[Obligation],
    now: i64,
) -> Result<Routed, OrgError> {
    let dir = Directory::load(c, ctx.org, now, ctx.web)?;
    let mut out = Routed {
        expires_at: now,
        ..Routed::default()
    };
    for o in obligations {
        if o.spec.reviewers == Reviewers::Requester {
            // The requester's own review: no routing, no escalation.
            out.expires_at = out.expires_at.max(now + i64::try_from(o.spec.window()).unwrap_or(0));
            out.notify.extend(dir.eligible(o, ctx.requester, None));
            continue;
        }
        let first = resolve(&dir, ctx, &o.spec.reviewers, false);
        let deadline = now + dir.window(&o.spec.reviewers, &o.spec);
        record_step(c, ctx.org, request, &o.id, 0, &o.spec.reviewers, &first, now, deadline)?;
        let mut set = first.members;
        let mut last_deadline = deadline;
        let chain = o.spec.chain();
        let mut step = 0usize;
        while dir.eligible(o, ctx.requester, Some(&set)).len() < usize::from(o.spec.quorum) && step < chain.len() {
            step += 1;
            let next = resolve(&dir, ctx, &chain[step - 1], true);
            let new: BTreeSet<String> = next.members.difference(&set).cloned().collect();
            if new.is_empty() {
                continue;
            }
            last_deadline = now + dir.window(&chain[step - 1], &o.spec);
            record_step(c, ctx.org, request, &o.id, step, &chain[step - 1], &next, now, last_deadline)?;
            audit::record(
                c,
                ctx.org,
                audit::kinds::REVIEW_ESCALATED,
                None,
                Some(request),
                &json!({"obligation": o.id, "step": step, "to": new, "at_submit": true}),
                now,
            )?;
            set.extend(new);
        }
        out.expires_at = out.expires_at.max(last_deadline);
        out.notify.extend(dir.eligible(o, ctx.requester, Some(&set)));
        if !set.is_empty() {
            out.resolved.insert(o.id.clone(), set);
        }
    }
    Ok(out)
}

/// Records one step and its audit event.
#[allow(clippy::too_many_arguments, reason = "one step's parts")]
pub fn record_step(
    c: &mut DbConnInner,
    org: &str,
    request: &str,
    obligation: &str,
    step: usize,
    reviewers: &Reviewers,
    res: &Resolution,
    now: i64,
    deadline: i64,
) -> Result<(), OrgError> {
    reins_org_routing::q_insert_step(
        c,
        &ReinsOrgStep {
            request_uuid: request.to_owned(),
            obligation: obligation.to_owned(),
            step: i32::try_from(step).unwrap_or(i32::MAX),
            reviewers: serde_json::to_string(reviewers).unwrap_or_default(),
            resolved: serde_json::to_string(&res.members).unwrap_or_else(|_| "[]".to_owned()),
            source: res.source.clone(),
            started_at: now,
            deadline,
        },
    )?;
    audit::record(
        c,
        org,
        audit::kinds::REVIEW_RESOLVED,
        None,
        Some(request),
        &json!({"obligation": obligation, "step": step, "members": res.members, "source": res.source,
                "evidence_hash": res.evidence, "unresolved": res.unresolved}),
        now,
    )?;
    Ok(())
}

/// Every dynamic reviewer set obligations may need, their own and their escalation chains'.
#[must_use]
pub fn dynamic_sets(obligations: &[Obligation]) -> Vec<Reviewers> {
    let mut out = Vec::new();
    for o in obligations {
        let chain = o.spec.chain();
        for r in std::iter::once(&o.spec.reviewers).chain(chain.iter()).flat_map(Reviewers::parts) {
            if matches!(r, Reviewers::CodeOwners | Reviewers::OnCall { .. }) && !out.contains(r) {
                out.push(r.clone());
            }
        }
    }
    out
}

/// What the request touches, for code owners (a PR merge's files through the reader, when it is one).
pub async fn touches(pool: &DbPool, org: &str, tool: &str, args: &Value) -> Touches {
    if let Some((repo, pr)) = merged_pr(tool, args) {
        let paths = codeowners::pr_files(pool, org, &repo, pr).await;
        return Touches {
            repo: Some(repo),
            paths,
        };
    }
    Touches {
        repo: repo_of(tool, args),
        paths: None,
    }
}

/// Asks each dynamic source what `sets` need.
pub async fn fetch(pool: &DbPool, org: &str, sets: &[Reviewers], touches: &Touches, now: i64) -> Dynamic {
    let mut out = Dynamic::new();
    for r in sets {
        let Some(key) = dynamic_key(r, touches) else {
            continue;
        };
        let res = match r {
            Reviewers::CodeOwners => match &touches.repo {
                Some(repo) => codeowners::owners(pool, org, repo, touches.paths.as_deref()).await,
                None => Resolution::nobody("codeowners", "no repository"),
            },
            Reviewers::OnCall {
                alias,
            } => oncall::current(pool, org, alias, now).await,
            _ => continue,
        };
        out.insert(key, res);
    }
    out
}

/// Asks the dynamic sources (code owners, on-call) what a request's obligations will need, before the decision
/// service's transaction. A source that fails or is slow answers nobody: its obligation escalates.
#[allow(clippy::too_many_arguments, reason = "one request's parts")]
pub async fn prefetch(
    pool: &DbPool,
    org: &str,
    member: &str,
    connection: &str,
    tool: &str,
    args: &Value,
    reason: Option<&str>,
    now: i64,
) -> (Touches, Dynamic) {
    let (o, member, t, a) = (org.to_owned(), member.to_owned(), tool.to_owned(), args.clone());
    let (cid, why) = (connection.to_owned(), reason.map(str::to_owned));
    let Ok(conn) = pool.get().await else {
        return (Touches::default(), Dynamic::new());
    };
    let needed = conn
        .run(move |c| -> Result<Vec<Reviewers>, OrgError> {
            let Some((policy, _)) = super::policy::active(c, &o)? else {
                return Ok(Vec::new());
            };
            let member_policy = super::decisions::member_policy_of(c, &o, &member)?;
            let v = evaluate_managed(
                &ManagedRequest {
                    org: &o,
                    member: &member,
                    connection: &cid,
                    tool: &t,
                    args: &a,
                    now,
                    reason: why.as_deref(),
                },
                &policy,
                member_policy.as_ref(),
            );
            Ok(if v.outcome == ManagedOutcome::Review {
                dynamic_sets(&v.obligations)
            } else {
                Vec::new()
            })
        })
        .await
        .unwrap_or_default();
    drop(conn);
    let touches = touches(pool, org, tool, args).await;
    if needed.is_empty() {
        return (touches, Dynamic::new());
    }
    let dynamic = fetch(pool, org, &needed, &touches, now).await;
    (touches, dynamic)
}

/// Applies an approved O3 admin change (inside [`super::admin`]'s transaction).
pub fn apply_admin(
    c: &mut DbConnInner,
    org: &str,
    action: &reins_proto::org::AdminAction,
    actor: &str,
    now: i64,
) -> Result<(), OrgError> {
    use reins_proto::org::AdminAction as A;
    let user_of = |c: &mut DbConnInner, membership: &str| -> Result<String, OrgError> {
        Ok(reins_org::q_membership_by_uuid(c, org, membership)?.ok_or(OrgError::NotFound)?.user_uuid.to_string())
    };
    match action {
        A::SetManager {
            member,
            manager,
        } => {
            let user = user_of(c, member)?;
            let manager = manager.as_deref().map(|m| user_of(c, m)).transpose()?;
            reins_org_routing::q_edit_person(c, org, &user, now, |p| {
                p.manager_source = manager.as_ref().map(|_| reins_org_routing::manager_source::ADMIN.to_owned());
                p.manager_user = manager;
            })?;
        }
        A::SetGroupRouting {
            group,
            backup,
            window_secs,
        } => reins_org_routing::q_save_group_setting(
            c,
            &reins_org_routing::ReinsOrgGroupSetting {
                org_uuid: org.to_owned(),
                group_uuid: group.clone(),
                backup_group: backup.clone(),
                window_secs: window_secs.and_then(|w| i64::try_from(w).ok()),
            },
        )?,
        A::SetGithubReader {
            app_id,
            installation_id,
        } => crate::db::models::reins_org_github_reader::q_set_github_reader(
            c,
            org,
            i64::try_from(*app_id).unwrap_or(0),
            i64::try_from(*installation_id).unwrap_or(0),
            now,
        )?,
        A::SetOnCallSource {
            alias,
            provider,
            schedule_id,
        } => reins_org_routing::q_set_oncall_source(c, org, alias, provider.as_str(), schedule_id)?,
        A::SetApprovalChannel {
            channel: kind,
            enabled,
            ..
        } => reins_org_routing::q_set_channel_enabled(c, org, kind.as_str(), *enabled)?,
        _ => return Err(OrgError::Internal("not a routing change".to_owned())),
    }
    audit::record(c, org, audit::kinds::ROUTING_CHANGED, Some(actor), None, &json!({"action": action}), now)?;
    Ok(())
}

/// After a sweep: tells the members an escalation reached (they must act), and updates channel messages.
pub async fn after_sweep(pool: &DbPool, swept: Vec<escalation::Swept>) {
    for s in swept {
        match s {
            escalation::Swept::Escalated(e) => {
                super::channels::update_all(pool, &e.request, Some(format!("Escalated to {}", e.label))).await;
                super::reviews::notify(pool, &e.to, &e.request);
            }
            escalation::Swept::Expired {
                request,
                ..
            } => super::channels::update_all(pool, &request, None).await,
        }
    }
}

/// The resources a request names, for resource tags.
#[must_use]
pub fn resources_of(policy: &OrgPolicy, req: &ManagedRequest<'_>) -> Vec<String> {
    let analysis = analyze_request(req.tool, req.args);
    reins_policy::org::resolve(req, &analysis, &policy.facts).parts
}

#[cfg(all(test, sqlite))]
pub(crate) mod tests {
    use std::collections::HashMap;

    use reins_proto::org::Role;

    use super::*;
    use crate::api::reins::org::{
        decisions::{self, Extra},
        kms::tests::use_test_kms,
        reviews::{
            self,
            tests::{Phone, approve_as, enroll},
        },
        testing::{add_group, add_org, db, member},
    };
    use crate::db::models::{reins_org_event, reins_org_request};

    pub type Phones = HashMap<String, Phone>;

    /// Org `o1`: Dana and Kim (Security admins), Lee (the requester), Mia, Fin1 and Fin2 (Finance), Ola; groups `sec`
    /// (Kim), `fin` (Fin1, Fin2, Lee), `sec-backup` (Ola). Every one but Dana has a phone that may decide. The policy
    /// adds `mgr` (Manager), `two-fin` (2 of Finance), `owners` (code owners), `oncall` (on-call: payments) and `prod`
    /// (sec), each on its own resource.
    pub fn routed_org(c: &mut DbConnInner) -> Phones {
        use_test_kms();
        add_org(c, "o1");
        member(c, "o1", "dana", &[Role::SecurityAdmin]);
        let kim = member(c, "o1", "kim", &[Role::SecurityAdmin]);
        let lee = member(c, "o1", "lee", &[]);
        member(c, "o1", "mia", &[]);
        let f1 = member(c, "o1", "fin1", &[]);
        let f2 = member(c, "o1", "fin2", &[]);
        let ola = member(c, "o1", "ola", &[]);
        add_group(c, "o1", "sec", &[&kim]);
        add_group(c, "o1", "fin", &[&f1, &f2, &lee]);
        add_group(c, "o1", "sec-backup", &[&ola]);
        let mut phones = Phones::new();
        for u in ["kim", "lee", "mia", "fin1", "fin2", "ola"] {
            phones.insert(u.to_owned(), enroll(c, "o1", u, &format!("{u}-phone"), Some(false), 100));
        }
        let rule = |id: &str, resource: &str, review: Value| {
            json!({"id": id, "issuer": "owner", "when": {"resource": resource}, "then": "review",
                   "explain": id, "review": review})
        };
        let rules = json!([
            rule("mgr", "acme/mgr", json!({"reviewers": "manager", "class": "deferred"})),
            rule("two-fin", "acme/fin", json!({"reviewers": {"group": "fin"}, "quorum": 2, "class": "deferred"})),
            rule("owners", "acme/code", json!({"reviewers": "code_owners", "class": "deferred"})),
            rule("oncall", "acme/pay", json!({"reviewers": {"on_call": {"alias": "payments"}}, "class": "deferred"})),
            rule("prod", "acme/prod", json!({"reviewers": {"group": "sec"}, "class": "deferred"})),
            rule(
                "esc",
                "acme/esc*",
                json!({"reviewers": {"group": "fin"}, "escalate": ["code_owners"],
                                            "class": "deferred"})
            ),
        ]);
        let mut p: Value = serde_json::from_str(crate::api::reins::org::policy::DEVELOPER_BASELINE).unwrap();
        p["rules"] = rules;
        p["facts"] = json!({"owners": {"acme": "work"}});
        let (rev, hash) = crate::api::reins::org::policy::save_draft(c, "o1", "dana", &p.to_string(), 10).unwrap();
        crate::api::reins::org::policy::apply_activation(c, "o1", rev, &hash, "dana", 11).unwrap();
        phones
    }

    pub fn set_manager(c: &mut DbConnInner, user: &str, manager: &str) {
        reins_org_routing::q_edit_person(c, "o1", user, 50, |p| {
            p.manager_user = Some(manager.to_owned());
            p.manager_source = Some(reins_org_routing::manager_source::DIRECTORY.to_owned());
        })
        .unwrap();
    }

    pub fn ask_with(c: &mut DbConnInner, repo: &str, dynamic: &Dynamic, now: i64) -> decisions::Submitted {
        let touches = Touches {
            repo: Some(repo.to_owned()),
            paths: None,
        };
        decisions::submit_with(
            c,
            "o1",
            "lee",
            "conn-lee",
            "Laptop",
            "github_create_issue",
            &json!({"repo": repo, "title": "x"}),
            None,
            &Extra {
                reason: None,
                touches: &touches,
                dynamic,
            },
            now,
        )
        .unwrap()
    }

    pub fn resolved_of(c: &mut DbConnInner, id: &str) -> ResolvedSets {
        stored(reins_org_request::q_request(c, "o1", id).unwrap().unwrap().resolved.as_deref())
    }

    pub fn approve(
        c: &mut DbConnInner,
        phones: &Phones,
        user: &str,
        id: &str,
        now: i64,
    ) -> Result<reviews::Answered, OrgError> {
        approve_as(c, &phones[user], user, id, now)
    }

    #[test]
    fn manager_is_from_the_directory_only() {
        let mut c = db();
        let phones = routed_org(&mut c);
        // No manager on record: nobody is resolved for the manager; it escalates to Security admins.
        let s = ask_with(&mut c, "acme/mgr", &Dynamic::new(), 200);
        assert!(!resolved_of(&mut c, &s.id).get("mgr").is_some_and(|m| m.contains("mia")));
        // A stored manager who isn't an active member doesn't count.
        set_manager(&mut c, "lee", "stranger");
        let s = ask_with(&mut c, "acme/mgr", &Dynamic::new(), 201);
        assert!(!resolved_of(&mut c, &s.id).get("mgr").is_some_and(|m| m.contains("stranger")));
        set_manager(&mut c, "lee", "mia");
        let s = ask_with(&mut c, "acme/mgr", &Dynamic::new(), 202);
        assert_eq!(resolved_of(&mut c, &s.id)["mgr"], ["mia".to_owned()].into());
        assert!(s.notify.contains(&"mia".to_owned()));
        assert!(matches!(approve(&mut c, &phones, "mia", &s.id, 203).unwrap(), reviews::Answered::Approved));
    }

    #[test]
    fn requester_is_never_a_reviewer() {
        let mut c = db();
        let phones = routed_org(&mut c);
        // Lee is in Finance and named as his own manager: neither makes him a reviewer.
        set_manager(&mut c, "lee", "lee");
        let s = ask_with(&mut c, "acme/mgr", &Dynamic::new(), 200);
        assert!(!resolved_of(&mut c, &s.id).values().any(|m| m.contains("lee")));
        assert!(!s.notify.contains(&"lee".to_owned()));
        assert!(approve(&mut c, &phones, "lee", &s.id, 201).is_err());
        let s = ask_with(&mut c, "acme/fin", &Dynamic::new(), 200);
        assert!(!s.notify.contains(&"lee".to_owned()));
        assert!(approve(&mut c, &phones, "lee", &s.id, 201).is_err());
        // On-call naming the requester: dropped.
        let mut d = Dynamic::new();
        d.insert("oncall:payments".into(), Resolution::from(["lee".to_owned()].into(), "oncall:pagerduty"));
        let s = ask_with(&mut c, "acme/pay", &d, 200);
        assert!(!resolved_of(&mut c, &s.id).values().any(|m| m.contains("lee")));
        assert!(approve(&mut c, &phones, "lee", &s.id, 201).is_err());
    }

    #[test]
    fn an_unsatisfiable_obligation_escalates_never_widens() {
        let mut c = db();
        let phones = routed_org(&mut c);
        // The on-call API is down: nobody resolves, so it escalates at once (no manager on record) to Security
        // admins. Kim can decide; Dana has no phone.
        let s = ask_with(&mut c, "acme/pay", &Dynamic::new(), 200);
        assert_eq!(s.outcome, ManagedOutcome::Review);
        let r = resolved_of(&mut c, &s.id);
        assert_eq!(r["oncall"], ["kim".to_owned()].into(), "{r:?}");
        assert_eq!(s.notify, ["kim".to_owned()]);
        let steps = reins_org_routing::q_steps(&mut c, &s.id).unwrap();
        assert_eq!(steps.iter().map(|s| s.step).collect::<Vec<_>>(), [0, 2]);
        // Kim loses her phone: nobody can approve, and nothing falls to the requester. At the deadline it expires.
        reins_org::q_revoke_devices(&mut c, "o1", "kim", 201).unwrap();
        assert!(approve(&mut c, &phones, "lee", &s.id, 202).is_err());
        assert!(approve(&mut c, &phones, "kim", &s.id, 202).is_err());
        let row = reins_org_request::q_request(&mut c, "o1", &s.id).unwrap().unwrap();
        decisions::sweep(&mut c, row.expires_at).unwrap();
        let row = reins_org_request::q_request(&mut c, "o1", &s.id).unwrap().unwrap();
        assert_eq!(row.state, reins_org_request::request_state::EXPIRED);
        assert!(row.decision.is_none(), "never approved");
    }

    #[test]
    fn posture_failing_reviewers_are_skipped() {
        let mut c = db();
        routed_org(&mut c);
        set_manager(&mut c, "lee", "mia");
        reins_org::q_revoke_devices(&mut c, "o1", "mia", 150).unwrap();
        enroll(&mut c, "o1", "mia", "mia-rooted", Some(true), 150);
        let s = ask_with(&mut c, "acme/mgr", &Dynamic::new(), 200);
        let r = resolved_of(&mut c, &s.id);
        assert!(!r.get("mgr").is_some_and(|m| m.contains("mia")), "{r:?}");
        assert!(!s.notify.contains(&"mia".to_owned()));
        // It escalated instead: Security admins with a phone (Kim).
        assert!(s.notify.contains(&"kim".to_owned()));
    }

    #[test]
    fn two_of_finance_needs_two_people() {
        let mut c = db();
        let phones = routed_org(&mut c);
        let s = ask_with(&mut c, "acme/fin", &Dynamic::new(), 200);
        assert_eq!(s.notify, ["fin1".to_owned(), "fin2".to_owned()]);
        assert!(matches!(approve(&mut c, &phones, "fin1", &s.id, 201).unwrap(), reviews::Answered::Recorded));
        assert!(approve(&mut c, &phones, "fin1", &s.id, 202).is_err(), "once");
        let items = reviews::items_for(&mut c, "o1", "fin2", 203).unwrap();
        assert_eq!(items[0].progress, "1 of 2");
        assert_eq!(items[0].needs, ["2 of Group fin"]);
        assert!(matches!(approve(&mut c, &phones, "fin2", &s.id, 204).unwrap(), reviews::Answered::Approved));
    }

    #[test]
    fn every_resolution_is_audited_with_evidence() {
        let mut c = db();
        let phones = routed_org(&mut c);
        let mut d = Dynamic::new();
        d.insert(
            "oncall:payments".into(),
            Resolution {
                members: ["ola".to_owned()].into(),
                unresolved: Vec::new(),
                source: "oncall:pagerduty".into(),
                evidence: "abc123".into(),
            },
        );
        let s = ask_with(&mut c, "acme/pay", &d, 200);
        let events = reins_org_event::q_events(&mut c, "o1", None, 100).unwrap();
        let resolved: Vec<Value> = events
            .iter()
            .filter(|e| e.kind == "review.resolved" && e.subject.as_deref() == Some(&s.id))
            .map(|e| serde_json::from_str(&e.detail).unwrap())
            .collect();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0]["obligation"], "oncall");
        assert_eq!(resolved[0]["members"], json!(["ola"]));
        assert_eq!(resolved[0]["source"], "oncall:pagerduty");
        assert_eq!(resolved[0]["evidence_hash"], "abc123");
        // The countersigned decision carries the resolution.
        approve(&mut c, &phones, "ola", &s.id, 201).unwrap();
        let row = reins_org_request::q_request(&mut c, "o1", &s.id).unwrap().unwrap();
        let env: reins_proto::managed::Envelope = serde_json::from_str(row.decision.as_deref().unwrap()).unwrap();
        let record: reins_proto::managed::DecisionRecord = env.parse().unwrap();
        assert_eq!(record.resolved["oncall"], ["ola".to_owned()]);
    }

    #[test]
    fn the_roster_carries_managers() {
        let mut c = db();
        routed_org(&mut c);
        set_manager(&mut c, "lee", "mia");
        set_manager(&mut c, "kim", "stranger");
        reins_org_routing::q_save_group_setting(
            &mut c,
            &reins_org_routing::ReinsOrgGroupSetting {
                org_uuid: "o1".into(),
                group_uuid: "sec".into(),
                backup_group: Some("sec-backup".into()),
                window_secs: None,
            },
        )
        .unwrap();
        let r = crate::api::reins::org::roster::build(&mut c, "o1", None, 300).unwrap();
        assert_eq!(r.member("lee").unwrap().manager.as_deref(), Some("mia"));
        assert_eq!(r.member("kim").unwrap().manager, None, "not a member");
        assert_eq!(r.backups.get("sec").map(String::as_str), Some("sec-backup"));
    }

    #[test]
    fn a_team_manager_is_set_by_two_admins_on_their_phones() {
        use reins_proto::org::{AdminAction, Tier};
        let mut c = db();
        let phones = routed_org(&mut c);
        let action = AdminAction::SetManager {
            member: "m-lee".into(),
            manager: Some("m-mia".into()),
        };
        // Business has a directory: refused.
        let mut t = reins_org::q_org(&mut c, "o1").unwrap().unwrap();
        t.tier = Tier::Business.as_str().to_owned();
        reins_org::q_save_org(&mut c, &t).unwrap();
        assert!(crate::api::reins::org::admin::propose(&mut c, "o1", "dana", &action, None, 300).is_err());
        t.tier = Tier::Team.as_str().to_owned();
        reins_org::q_save_org(&mut c, &t).unwrap();
        let (crate::api::reins::org::admin::Proposed::Pending(id), _) =
            crate::api::reins::org::admin::propose(&mut c, "o1", "dana", &action, None, 300).unwrap()
        else {
            panic!("a change that takes device signatures waits")
        };
        assert!(reins_org_routing::q_person(&mut c, "o1", "lee").unwrap().is_none());
        let dana = enroll(&mut c, "o1", "dana", "dana-phone", Some(false), 300);
        reviews::tests::change_answer_as(
            &mut c,
            &dana,
            "dana",
            &id,
            reins_proto::managed::ReviewDecision::Propose,
            301,
        )
        .unwrap();
        reviews::tests::change_answer_as(
            &mut c,
            &phones["kim"],
            "kim",
            &id,
            reins_proto::managed::ReviewDecision::Approve,
            302,
        )
        .unwrap();
        let p = reins_org_routing::q_person(&mut c, "o1", "lee").unwrap().unwrap();
        assert_eq!((p.manager_user.as_deref(), p.manager_source.as_deref()), (Some("mia"), Some("admin")));
    }
}
