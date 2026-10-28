//! A Managed account's MCP calls go to its org's decision service, never to a phone (`docs/design/decisions-v2/05` §5:
//! "Phones in Managed mode are approval devices only"; `08` R6: requests carry their connection's profile).
//!
//! Which account a connection belongs to decides it: a connection of an active member of an org is Managed, every
//! other connection is Personal and takes the relay as before. The tools offered are the ones the org's executors
//! run through its managed connections.

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{OrgError, decisions, enabled, leases::managed_org};
use crate::{
    CONFIG,
    api::reins::{
        mcp::{self, Action, McpHeaders},
        mcp_routes::{check_connection_rate, enter_waiting},
        now_unix,
        tools::{self, ToolArgError, ToolInvocation},
    },
    db::{DbConn, DbPool, models::ReinsConnection},
};

/// The tools an org executor runs (its GitHub App and Google Workspace connections), and the result tool.
pub const MANAGED_TOOLS: &[&str] = &[
    "github_api_read",
    "github_api_write",
    "github_create_issue",
    "github_comment",
    "gmail_send",
    "calendar_list_events",
    "calendar_create_event",
    "reins_get_result",
];

/// The org of a Managed connection, if it is one (organizations on, its account an active member).
pub async fn org_of(connection: &ReinsConnection, conn: &DbConn) -> Option<String> {
    if !enabled() {
        return None;
    }
    let user = connection.user_uuid.to_string();
    conn.run(move |c| managed_org(c, &user)).await.ok().flatten()
}

/// The tools a listing keeps: the org's; for a family member (P7), only those of the accounts a guardian connected.
fn only_managed(mut body: Value, family: Option<&[&str]>) -> Value {
    let tools = body.pointer_mut("/result/tools").and_then(Value::as_array_mut);
    if let Some(tools) = tools {
        tools.retain(|t| t["name"].as_str().is_some_and(|n| offered(n, family)));
    }
    body
}

fn offered(name: &str, family: Option<&[&str]>) -> bool {
    MANAGED_TOOLS.contains(&name) && family.is_none_or(|f| f.contains(&name))
}

fn tool_error(message: &str) -> Value {
    tools::failure(message)
}

/// What the agent is told for a request's status.
fn render(status: &Value) -> Value {
    match status["status"].as_str() {
        Some("done") => match &status["result"] {
            r @ Value::Object(_) => tools::success(r),
            other => tools::success(&json!({"result": other})),
        },
        Some("pending_review" | "approved") => tools::success(status),
        _ => tool_error(status["reason"].as_str().unwrap_or("Not done.")),
    }
}

/// Answers one `/mcp` POST of a Managed connection: `(HTTP status, body)`.
pub async fn post(
    org: &str,
    headers: &McpHeaders,
    body: &[u8],
    connection: &ReinsConnection,
    conn: DbConn,
    pool: &DbPool,
) -> (u16, Option<Value>) {
    let services = ["github".to_owned(), "gmail".to_owned(), "calendar".to_owned()];
    let (o, u) = (org.to_owned(), connection.user_uuid.to_string());
    let family = match conn.run(move |c| super::family_connect::member_tools(c, &o, &u)).await {
        Ok(f) => f,
        Err(e) => {
            warn!("Family tools of {org}: {e:?}");
            Some(vec!["reins_get_result"])
        }
    };
    let action = mcp::handle(headers, body, Some(&services), &[]);
    let (id, name, arguments, modern, reason) = match action {
        Action::Reply {
            status,
            body,
        } => return (status, body.map(|b| only_managed(b, family.as_deref()))),
        // Managed calls are decided by the org's service; a deferral hint (P4's tickets) is not theirs.
        Action::CallTool {
            id,
            name,
            arguments,
            modern,
            reason,
            ..
        } => (id, name, arguments, modern, reason),
        // Ticket resources are the personal tickets' (P4); a managed connection's deferred decisions are the org's.
        Action::Resource {
            id,
            method,
            ..
        } => return (404, Some(mcp::rpc_error(&id, -32601, &format!("Method not found: {method}"), None))),
    };
    if let Err(wait) = check_connection_rate(connection) {
        return (
            200,
            Some(mcp::tool_reply(
                &id,
                modern,
                tool_error(&format!("Too many requests. Try again in {}s.", wait.as_secs().max(1))),
            )),
        );
    }
    let reply = |result: Value| (200, Some(mcp::tool_reply(&id, modern, result)));
    if !offered(&name, family.as_deref()) {
        return (400, Some(mcp::rpc_error(&id, -32602, &format!("Unknown tool: {name}"), None)));
    }
    let (org, connection_id, user) = (org.to_owned(), connection.uuid.clone(), connection.user_uuid.to_string());
    let request_id = match managed_invocation(&name, &arguments) {
        Err(ToolArgError::UnknownTool(t)) => {
            return (400, Some(mcp::rpc_error(&id, -32602, &format!("Unknown tool: {t}"), None)));
        }
        Err(ToolArgError::Invalid(m)) => return reply(tool_error(&m)),
        // The personal ticket tools (P4) aren't managed tools.
        Ok(
            ToolInvocation::Wait(..) | ToolInvocation::Cancel(_) | ToolInvocation::Tickets | ToolInvocation::Reply(..),
        ) => {
            return (400, Some(mcp::rpc_error(&id, -32602, &format!("Unknown tool: {name}"), None)));
        }
        Ok(ToolInvocation::GetResult(r)) => r.0,
        Ok(ToolInvocation::Relay(..)) => {
            let now = now_unix();
            // Code owners and on-call are asked before the decision service's transaction (never by the agent).
            let (touches, dynamic) =
                super::routing::prefetch(pool, &org, &user, &connection_id, &name, &arguments, reason.as_deref(), now)
                    .await;
            let (o, member, cid, label, tool, args) = (
                org.clone(),
                user.clone(),
                connection_id.clone(),
                connection.label.clone(),
                name.clone(),
                arguments.clone(),
            );
            let submitted = conn
                .run(move |c| {
                    let extra = decisions::Extra {
                        reason: reason.as_deref(),
                        touches: &touches,
                        dynamic: &dynamic,
                    };
                    decisions::submit_with(c, &o, &member, &cid, &label, &tool, &args, None, &extra, now)
                })
                .await;
            match submitted {
                Ok(s) => {
                    super::reviews::notify(pool, &s.notify, &s.id);
                    if s.outcome == reins_policy::org::ManagedOutcome::Review && !s.instant {
                        // Deferred: the agent carries on with its ticket (03 §4.1).
                        let (o, cid, rid) = (org.clone(), connection_id.clone(), s.id.clone());
                        return match conn.run(move |c| decisions::status(c, &o, &cid, &rid, now_unix())).await {
                            Ok(st) => reply(render(&st)),
                            Err(e) => reply(tool_error(e.message())),
                        };
                    }
                    s.id
                }
                Err(OrgError::Invalid(m)) => return reply(tool_error(m)),
                Err(e) => {
                    warn!("Org decision for {org}: {e:?}");
                    return reply(tool_error("Your organization couldn't decide this now. Try again."));
                }
            }
        }
    };
    // Wait for the answer up to the relay's wait, like a Personal call waits for the phone: within the connection's
    // cap on waiting calls, and without holding a pooled database connection while waiting.
    drop(conn);
    let _waiting = match enter_waiting(connection) {
        Ok(place) => place,
        Err(wait) => {
            return reply(tool_error(&format!(
                "Too many calls of this connection are waiting. Try again in {}s.",
                wait.as_secs().max(1)
            )));
        }
    };
    let deadline = Instant::now() + Duration::from_secs(CONFIG.reins_relay_wait_secs());
    let fetch = || {
        let (o, cid, rid) = (org.clone(), connection_id.clone(), request_id.clone());
        async move {
            let conn = pool.get().await.map_err(|e| OrgError::Internal(format!("{e:?}")))?;
            conn.run(move |c| decisions::status(c, &o, &cid, &rid, now_unix())).await
        }
    };
    match wait_final(&request_id, deadline, fetch).await {
        Ok(status) => reply(render(&status)),
        Err(OrgError::NotFound) => reply(tool_error("No such request on this connection.")),
        Err(e) => reply(tool_error(e.message())),
    }
}

/// Polls `poll` until its status is final or `deadline` passes, waking early when the request changes.
pub async fn wait_final<F, Fut>(request_id: &str, deadline: Instant, mut poll: F) -> Result<Value, OrgError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Value, OrgError>>,
{
    loop {
        let status = poll().await?;
        if decisions::is_final(&status) || Instant::now() >= deadline {
            return Ok(status);
        }
        decisions::wait_change(request_id, deadline.saturating_duration_since(Instant::now())).await;
    }
}

#[cfg(all(test, sqlite))]
mod tests {
    use super::*;
    use crate::api::reins::org::{decisions::tests::pilot, testing::db};

    #[test]
    fn only_managed_accounts_are_routed_to_the_org() {
        let mut c = db();
        pilot(&mut c);
        assert_eq!(managed_org(&mut c, "lee").unwrap().as_deref(), Some("o1"));
        // A Personal account (no org), and a suspended member.
        crate::api::reins::org::testing::add_user(&mut c, "bob", "bob@example.com");
        assert_eq!(managed_org(&mut c, "bob").unwrap(), None);
        crate::api::reins::org::testing::set_state(
            &mut c,
            "o1",
            "lee",
            crate::db::models::reins_org::state::SUSPENDED_BY_DIRECTORY,
        );
        assert_eq!(managed_org(&mut c, "lee").unwrap(), None);
    }

    #[test]
    fn tools_list_offers_only_managed_tools() {
        let body = json!({"jsonrpc": "2.0", "id": 1, "result": {"tools": tools::tool_definitions()}});
        let names: Vec<String> = only_managed(body.clone(), None)["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .collect();
        assert!(names.iter().all(|n| MANAGED_TOOLS.contains(&n.as_str())), "{names:?}");
        assert!(names.contains(&"github_create_issue".to_owned()) && names.contains(&"gmail_send".to_owned()));
        assert!(!names.iter().any(|n| n.starts_with("vault_") || n.starts_with("telegram_")));
        // A family member (P7): only the tools of the accounts a guardian connected.
        let family = ["github_create_issue", "reins_get_result"];
        let names: Vec<String> = only_managed(body, Some(&family))["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .collect();
        assert!(names.iter().all(|n| family.contains(&n.as_str())), "{names:?}");
        assert!(!offered("gmail_send", Some(&family)));
        assert!(offered("gmail_send", None));
    }

    #[test]
    fn a_deferred_review_renders_the_ticket_object() {
        let mut c = db();
        pilot(&mut c);
        let s = decisions::tests::ask(&mut c, &decisions::tests::issue("acme/prod-db"), 100);
        let status = decisions::status(&mut c, "o1", "conn-lee", &s.id, 101).unwrap();
        let rendered = render(&status);
        assert_ne!(rendered["isError"], json!(true));
        let structured = &rendered["structuredContent"];
        assert_eq!(structured["status"], "pending_review");
        assert_eq!(structured["ticket"], s.id);
    }
}

#[cfg(test)]
mod wait_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[tokio::test]
    async fn waiting_returns_as_soon_as_the_answer_is_final_and_stops_at_the_deadline() {
        let polls = AtomicUsize::new(0);
        let deadline = Instant::now() + Duration::from_secs(30);
        let done = wait_final("or_w1", deadline, || {
            let n = polls.fetch_add(1, Ordering::SeqCst);
            async move {
                if n < 2 {
                    Ok(json!({"status": "approved"}))
                } else {
                    Ok(json!({"status": "done", "result": {}}))
                }
            }
        });
        let woke = async {
            for _ in 0..2 {
                tokio::time::sleep(Duration::from_millis(20)).await;
                decisions::wake("or_w1");
            }
        };
        let (status, ()) = tokio::join!(done, woke);
        assert_eq!(status.unwrap()["status"], "done");
        assert_eq!(polls.load(Ordering::SeqCst), 3);
        // Past the deadline it answers with what it has.
        let past = Instant::now();
        let status = wait_final("or_w2", past, async || Ok(json!({"status": "pending_review"}))).await.unwrap();
        assert_eq!(status["status"], "pending_review");
        // An error ends the wait.
        assert!(wait_final("or_w3", deadline, async || { Err(OrgError::NotFound) }).await.is_err());
    }
}

/// [`tools::parse_invocation_with`] for a managed connection, except `reins_get_result`: an org request's id
/// (`or_<uuid>`) is not a personal ticket's (`rt_<request>`, P4), so it is read as given, up to 64 characters.
fn managed_invocation(name: &str, arguments: &Value) -> Result<ToolInvocation, ToolArgError> {
    if name != "reins_get_result" {
        return tools::parse_invocation_with(name, arguments, &[]);
    }
    let invalid = |m: &str| Err(ToolArgError::Invalid(m.to_owned()));
    let Some(args) = arguments.as_object() else {
        return invalid("Arguments must be an object.");
    };
    if args.keys().any(|k| k != "request_id") {
        return invalid("Only `request_id` is accepted.");
    }
    match args.get("request_id").and_then(Value::as_str) {
        Some(id) if !id.is_empty() && id.len() <= 64 => {
            Ok(ToolInvocation::GetResult(reins_proto::ids::RequestId(id.to_owned())))
        }
        _ => invalid("`request_id` is not valid."),
    }
}
