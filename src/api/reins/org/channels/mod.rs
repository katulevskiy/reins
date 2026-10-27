//! Slack and Teams review notifications (05 §6). They are **notifications with a link, never a decision path**:
//!
//! - a member a review is routed to gets one line (the `minimal` level: requester, agent, effect kind, resource,
//!   level, what it needs) and, for L0–L2 only, a button to the signed web approval ([`super::web_approval`]). L3
//!   says "Open the Reins app" and has no link;
//! - request bodies never leave Reins (no title, no message text, no arguments);
//! - inbound webhooks are verified and answered, and never decide anything;
//! - on a decision, expiry or escalation every posted message is updated ("Approved", "Expired", "Escalated to
//!   Security").
//!
//! A channel is the org's own app (a Slack app, a single-tenant Azure bot): its secrets are posted by a Connection
//! admin and sealed under the tenant's data key; turning it on is a two-person admin change
//! (`AdminAction::SetApprovalChannel`).

pub mod slack;
pub mod teams;

use reins_policy::org::{ManagedRequest, Obligation, analyze_request};
use reins_proto::{managed::channel_allowed, org::ChannelKind};
use serde_json::Value;
use zeroize::Zeroizing;

use super::{OrgError, kms, reviews, routing, web_approval};
use crate::db::{
    DbConnInner, DbPool,
    models::{
        reins_org_request::{self, ReinsOrgRequest, request_state as st},
        reins_org_routing::{self as store, ReinsOrgPosted},
    },
};

fn aad(org: &str, kind: ChannelKind) -> String {
    format!("{org}/channel/{}", kind.as_str())
}

/// Seals and stores a channel's configuration (JSON) as posted by a Connection admin. It stays off until a two-person
/// change turns it on.
#[cfg_attr(not(test), allow(dead_code, reason = "the console's channels page installs them"))]
pub fn install(c: &mut DbConnInner, org: &str, kind: ChannelKind, config: &Value, now: i64) -> Result<(), OrgError> {
    let ok = match kind {
        ChannelKind::Slack => slack::SlackConfig::from_value(config).is_some(),
        ChannelKind::Teams => teams::TeamsConfig::from_value(config).is_some(),
    };
    if !ok {
        return Err(OrgError::Invalid("Fill in every field."));
    }
    let text = Zeroizing::new(config.to_string());
    let (key_id, sealed) = kms::seal_data(c, org, &aad(org, kind), text.as_bytes(), now)?;
    if store::q_channel_for_use(c, org, kind.as_str())?.is_none() {
        store::q_set_channel_enabled(c, org, kind.as_str(), false)?;
    }
    store::q_set_channel_secret(c, org, kind.as_str(), &key_id, &sealed, now)?;
    super::audit::record(
        c,
        org,
        super::audit::kinds::CHANNEL_INSTALLED,
        None,
        None,
        &serde_json::json!({"kind": kind.as_str()}),
        now,
    )?;
    Ok(())
}

/// A channel's configuration, when installed and on (`any` also takes one that is off: for inbound verification).
pub fn config(
    c: &mut DbConnInner,
    org: &str,
    kind: ChannelKind,
    any: bool,
) -> Result<Option<Zeroizing<String>>, OrgError> {
    let Some(row) = store::q_channel_for_use(c, org, kind.as_str())? else {
        return Ok(None);
    };
    let (Some(key_id), Some(sealed)) = (row.key_id.as_deref(), row.sealed.as_deref()) else {
        return Ok(None);
    };
    if !row.enabled && !any {
        return Ok(None);
    }
    let plain = kms::open_data(c, org, key_id, &aad(org, kind), sealed)?;
    Ok(Some(Zeroizing::new(
        String::from_utf8(plain.to_vec()).map_err(|_| OrgError::Internal("channel config".into()))?,
    )))
}

/// The one line a channel carries (05 §6's `minimal` level): who asked, through which agent, what kind of effect on
/// which resource, the level, and who must decide. Never a body.
#[must_use]
pub fn line(
    requester: &str,
    agent: &str,
    kinds: &[String],
    resource: Option<&str>,
    level: &str,
    needs: &[String],
) -> String {
    let what = match resource {
        Some(r) => format!("{} {r}", kinds.join(", ")),
        None => kinds.join(", "),
    };
    let mut out = format!("{requester} · {agent} · {what} · {level}");
    if !needs.is_empty() {
        out.push_str(&format!(" · needs {}", needs.join(", ")));
    }
    out
}

/// What a channel message says about one request: the line, the deadline, and whether a web link may be offered.
#[derive(Clone, Debug)]
pub struct Message {
    pub line: String,
    pub due: i64,
    /// L0–L2: the web approval may be offered.
    pub web: bool,
}

/// The message for a request, from its stored trace and sealed call (only its effect kinds and resource are read).
pub fn message_of(c: &mut DbConnInner, row: &ReinsOrgRequest) -> Result<Message, OrgError> {
    let trace: Value = serde_json::from_str(&row.trace).unwrap_or_default();
    let level = trace["level"].as_str().unwrap_or_default().to_owned();
    let body = reins_org_request::q_body(c, &row.org_uuid, &row.uuid, "request")?.ok_or(OrgError::NotFound)?;
    let plain = kms::open_data(c, &row.org_uuid, &body.key_id, &format!("{}/request", row.uuid), &body.sealed)?;
    let call: Value = serde_json::from_slice(&plain).map_err(|e| OrgError::Internal(e.to_string()))?;
    let (tool, args) = (call["tool"].as_str().unwrap_or_default().to_owned(), call["args"].clone());
    let analysis = analyze_request(&tool, &args);
    let mut kinds: Vec<String> = analysis.effects.iter().map(|e| format!("{:?}", e.kind())).collect();
    kinds.dedup();
    let policy = super::policy::active(c, &row.org_uuid)?.map(|(p, _)| p);
    let resource = policy.as_ref().and_then(|p| {
        let req = ManagedRequest {
            org: &row.org_uuid,
            member: &row.user_uuid,
            connection: &row.connection_uuid,
            tool: &tool,
            args: &args,
            now: row.created_at,
            reason: None,
        };
        reins_policy::org::resolve(&req, &analysis, &p.facts).resource
    });
    let requester = reviews::email_of_user(c, &row.user_uuid)?;
    let agent = reviews::connection_label_of(c, &row.connection_uuid)?;
    let obligations: Vec<Obligation> = serde_json::from_str(&row.obligations).unwrap_or_default();
    let mut needs = Vec::new();
    for o in &obligations {
        needs.push(routing::label(c, &row.org_uuid, &o.spec.reviewers)?);
    }
    Ok(Message {
        line: line(&requester, &agent, &kinds, resource.as_deref(), &level, &needs),
        due: row.expires_at,
        web: channel_allowed(&level),
    })
}

/// One channel's messages for one org.
#[rocket::async_trait]
pub trait ApprovalChannel: Send + Sync {
    /// The channel's id for the member with `email` (a Slack user, a Teams conversation), linking them if needed.
    async fn link(&self, email: &str) -> Result<String, String>;
    /// Posts `msg` to `to` with the web approval `url` (none for L3); returns the message's reference.
    async fn post(&self, to: &str, msg: &Message, url: Option<&str>) -> Result<String, String>;
    /// Replaces a posted message with `outcome`.
    async fn update(&self, external_ref: &str, outcome: &str) -> Result<(), String>;
}

fn client_of(kind: ChannelKind, config: &str) -> Option<Box<dyn ApprovalChannel>> {
    let v: Value = serde_json::from_str(config).ok()?;
    match kind {
        ChannelKind::Slack => slack::SlackConfig::from_value(&v).map(|c| {
            let b: Box<dyn ApprovalChannel> = Box::new(slack::Slack::new(c));
            b
        }),
        ChannelKind::Teams => teams::TeamsConfig::from_value(&v).map(|c| {
            let b: Box<dyn ApprovalChannel> = Box::new(teams::Teams::new(c));
            b
        }),
    }
}

/// Who to post to for one request: (member, email, link token when L0–L2), for each enabled channel.
struct Plan {
    org: String,
    msg: Message,
    channels: Vec<(ChannelKind, Zeroizing<String>)>,
    to: Vec<(String, String, Option<String>)>,
}

fn plan(c: &mut DbConnInner, request: &str, members: &[String], now: i64) -> Result<Option<Plan>, OrgError> {
    let Some(row) = reins_org_request::q_request_any(c, request)? else {
        return Ok(None);
    };
    if row.state != st::REVIEWING {
        return Ok(None);
    }
    let org = row.org_uuid.clone();
    let mut channels = Vec::new();
    for kind in ChannelKind::ALL {
        if let Some(cfg) = config(c, &org, kind, false)? {
            channels.push((kind, cfg));
        }
    }
    if channels.is_empty() {
        return Ok(None);
    }
    let msg = message_of(c, &row)?;
    let mut to = Vec::new();
    for m in members.iter().filter(|m| **m != row.user_uuid) {
        let email = reviews::email_of_user(c, m)?;
        // The link is issued only to a member the review is routed to, and only for L0–L2.
        let token = if msg.web {
            web_approval::issue_token(c, &org, request, m, now)?
        } else {
            None
        };
        to.push((m.clone(), email, token));
    }
    Ok(Some(Plan {
        org,
        msg,
        channels,
        to,
    }))
}

/// Posts a review to the members it reached, in every channel the org turned on (after the push).
pub async fn fan_out(pool: &DbPool, request: &str, members: &[String]) {
    let (r, m, now) = (request.to_owned(), members.to_vec(), crate::api::reins::now_unix());
    let Ok(conn) = pool.get().await else {
        return;
    };
    let Ok(Some(plan)) = conn.run(move |c| plan(c, &r, &m, now)).await else {
        return;
    };
    drop(conn);
    for (kind, cfg) in &plan.channels {
        let Some(client) = client_of(*kind, cfg) else {
            continue;
        };
        for (member, email, token) in &plan.to {
            let target = match linked(pool, &plan.org, member, *kind).await {
                Some(t) => t,
                None => match client.link(email).await {
                    Ok(t) => {
                        remember_link(pool, &plan.org, member, *kind, &t).await;
                        t
                    }
                    Err(e) => {
                        info!("Org {}: {} link: {e}", plan.org, kind.as_str());
                        continue;
                    }
                },
            };
            let url = token.as_deref().map(web_approval::token_url);
            match client.post(&target, &plan.msg, url.as_deref()).await {
                Ok(external_ref) => {
                    if let Ok(conn) = pool.get().await {
                        let row = ReinsOrgPosted {
                            request_uuid: request.to_owned(),
                            user_uuid: member.clone(),
                            channel: kind.as_str().to_owned(),
                            external_ref,
                            posted_at: now,
                        };
                        drop(conn.run(move |c| store::q_insert_posted(c, &row)).await);
                    }
                }
                Err(e) => info!("Org {}: {} post: {e}", plan.org, kind.as_str()),
            }
        }
    }
}

async fn linked(pool: &DbPool, org: &str, member: &str, kind: ChannelKind) -> Option<String> {
    let conn = pool.get().await.ok()?;
    let (o, m) = (org.to_owned(), member.to_owned());
    let p = conn.run(move |c| store::q_person(c, &o, &m)).await.ok().flatten()?;
    match kind {
        ChannelKind::Slack => p.slack_user,
        ChannelKind::Teams => p.teams_conversation,
    }
}

async fn remember_link(pool: &DbPool, org: &str, member: &str, kind: ChannelKind, target: &str) {
    let Ok(conn) = pool.get().await else {
        return;
    };
    let (o, m, t) = (org.to_owned(), member.to_owned(), target.to_owned());
    let now = crate::api::reins::now_unix();
    drop(
        conn.run(move |c| {
            store::q_edit_person(c, &o, &m, now, |p| match kind {
                ChannelKind::Slack => p.slack_user = Some(t),
                ChannelKind::Teams => p.teams_conversation = Some(t),
            })
        })
        .await,
    );
}

/// What a posted message becomes once a request moved on.
pub fn outcome_of(c: &mut DbConnInner, row: &ReinsOrgRequest) -> Result<Option<String>, OrgError> {
    Ok(match row.state.as_str() {
        st::REVIEWING => None,
        st::DENIED => Some("Denied".to_owned()),
        st::EXPIRED => Some("Expired".to_owned()),
        st::CANCELLED => Some("Cancelled".to_owned()),
        _ => {
            let by = reins_org_request::q_approvals(c, &row.org_uuid, &row.uuid)?
                .into_iter()
                .rfind(|a| a.decision == "approve")
                .map(|a| reviews::email_of_user(c, &a.user_uuid))
                .transpose()?;
            Some(by.map_or_else(|| "Approved".to_owned(), |e| format!("Approved by {e}")))
        }
    })
}

/// Updates every message posted for `request` with `text`, or with its outcome when `text` is `None`.
pub async fn update_all(pool: &DbPool, request: &str, text: Option<String>) {
    let r = request.to_owned();
    let Ok(conn) = pool.get().await else {
        return;
    };
    let loaded = conn
        .run(
            move |c| -> Result<
                Option<(String, String, Vec<ReinsOrgPosted>, Vec<(ChannelKind, Zeroizing<String>)>)>,
                OrgError,
            > {
                let Some(row) = reins_org_request::q_request_any(c, &r)? else {
                    return Ok(None);
                };
                let Some(outcome) = text.or(outcome_of(c, &row)?) else {
                    return Ok(None);
                };
                let posted = store::q_posted_of(c, &r)?;
                let mut channels = Vec::new();
                for kind in ChannelKind::ALL {
                    if let Some(cfg) = config(c, &row.org_uuid, kind, true)? {
                        channels.push((kind, cfg));
                    }
                }
                Ok(Some((row.org_uuid, outcome, posted, channels)))
            },
        )
        .await;
    let Ok(Some((org, outcome, posted, channels))) = loaded else {
        return;
    };
    for (kind, cfg) in channels {
        let Some(client) = client_of(kind, &cfg) else {
            continue;
        };
        for p in posted.iter().filter(|p| p.channel == kind.as_str()) {
            if let Err(e) = client.update(&p.external_ref, &outcome).await {
                info!("Org {org}: {} update: {e}", kind.as_str());
            }
        }
    }
}

/// After an answer, a decision or the sweep: updates the posted messages in the background.
pub fn spawn_update(pool: &DbPool, request: &str, text: Option<String>) {
    let (pool, request) = (pool.clone(), request.to_owned());
    tokio::spawn(async move { update_all(&pool, &request, text).await });
}

/// After a review reached members: posts it in the org's channels in the background.
pub fn spawn_fan_out(pool: &DbPool, request: &str, members: &[String]) {
    let (pool, request, members) = (pool.clone(), request.to_owned(), members.to_vec());
    tokio::spawn(async move { fan_out(&pool, &request, &members).await });
}

/// The API base of a channel: its own, or (tests only, with `REINS_TEST_ALLOW_LOOPBACK`) the environment's.
#[must_use]
pub fn api_base(default: &str, env: &str) -> String {
    if crate::api::reins::outbound::allow_loopback()
        && let Ok(b) = std::env::var(env)
        && !b.trim().is_empty()
    {
        return b.trim().trim_end_matches('/').to_owned();
    }
    default.to_owned()
}

pub(crate) fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())
}

/// "Due 15:00 UTC".
#[must_use]
pub fn due_text(t: i64) -> String {
    chrono::DateTime::from_timestamp(t, 0).map(|d| d.format("Due %H:%M UTC").to_string()).unwrap_or_default()
}

#[cfg(all(test, sqlite))]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::api::reins::org::{
        routing::{Dynamic, tests::routed_org},
        testing::db,
    };

    #[test]
    fn messages_carry_no_body() {
        let mut c = db();
        routed_org(&mut c);
        let touches = routing::Touches::default();
        let s = crate::api::reins::org::decisions::submit_with(
            &mut c,
            "o1",
            "lee",
            "conn-lee",
            "Laptop",
            "github_create_issue",
            &json!({"repo": "acme/prod", "title": "Rotate the TOP-SECRET key", "body": "password hunter2"}),
            None,
            &crate::api::reins::org::decisions::Extra {
                reason: Some("because SECRET-REASON"),
                touches: &touches,
                dynamic: &Dynamic::new(),
            },
            200,
        )
        .unwrap();
        let row = reins_org_request::q_request(&mut c, "o1", &s.id).unwrap().unwrap();
        let m = message_of(&mut c, &row).unwrap();
        for secret in ["TOP-SECRET", "hunter2", "SECRET-REASON", "Rotate"] {
            assert!(!m.line.contains(secret), "{}", m.line);
        }
        assert!(m.line.contains("acme/prod") && m.line.contains("L") && m.line.contains("Group sec"), "{}", m.line);
        assert!(m.web);
    }

    #[test]
    fn install_takes_two_people() {
        use reins_proto::org::AdminAction;
        let mut c = db();
        let phones = routed_org(&mut c);
        let on = AdminAction::SetApprovalChannel {
            channel: ChannelKind::Slack,
            enabled: true,
        };
        // Not installed: refused.
        assert!(crate::api::reins::org::admin::propose(&mut c, "o1", "dana", &on, None, 300).is_err());
        install(&mut c, "o1", ChannelKind::Slack, &json!({"bot_token": "xoxb-1", "signing_secret": "s"}), 300).unwrap();
        assert!(install(&mut c, "o1", ChannelKind::Slack, &json!({"bot_token": "not-a-bot"}), 300).is_err());
        assert!(config(&mut c, "o1", ChannelKind::Slack, false).unwrap().is_none(), "installed, still off");
        let (crate::api::reins::org::admin::Proposed::Pending(id), _) =
            crate::api::reins::org::admin::propose(&mut c, "o1", "dana", &on, None, 301).unwrap()
        else {
            panic!("waits for a second admin")
        };
        let dana = reviews::tests::enroll(&mut c, "o1", "dana", "dana-phone", Some(false), 301);
        use crate::api::reins::org::reviews::tests::change_answer_as;
        use reins_proto::managed::ReviewDecision;
        change_answer_as(&mut c, &dana, "dana", &id, ReviewDecision::Propose, 302).unwrap();
        assert!(config(&mut c, "o1", ChannelKind::Slack, false).unwrap().is_none(), "one person isn't enough");
        change_answer_as(&mut c, &phones["kim"], "kim", &id, ReviewDecision::Approve, 303).unwrap();
        assert!(config(&mut c, "o1", ChannelKind::Slack, false).unwrap().is_some());
    }
}
