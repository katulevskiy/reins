//! How a call to another integration is handled and approved. It follows the Gmail flow: the call is fetched or
//! previewed, checked against the grants, answered at once when they cover it, or parked for the user, whose decision
//! (and standing permission, if any) then answers the AI. Everything is logged.

use std::collections::{BTreeMap, BTreeSet};

use super::{Item, items_json};
use crate::engine::Engine;
use crate::session::Session;
use crate::store::{AuditMessage, AuditRecord, unix_now};
use crate::types::{ApprovalChoice, StandingGrant};
use crate::views::{self, ParkedConnector, ParkedRequest};
use crate::{CoreError, text};
use reins_policy::{Grant, Scope, ServiceScope};
use reins_proto::connector::{ConnectorCall, Effect, normalize_account_name};
use reins_proto::desktop;
use reins_proto::ids::{ConnectionId, GrantId};
use reins_proto::relay::{RelayOutcome, RelayRequest, ToolResult};

/// How long a one-time pass lasts after the user approved a request the AI had already stopped waiting for.
const RETRY_PASS_SECS: i64 = 15 * 60;

/// The answer to a desktop-only call from anything but the desktop app paired with this connection.
const DESKTOP_ONLY: &str = "This must come from the Reins desktop app paired with this phone.";

/// What the AI is told when an integration fails (never a token or a URL).
fn ai_message(service: &str, e: &CoreError) -> String {
    let name = views::service_name(service);
    match e {
        CoreError::Service {
            reason,
        } => reason.clone(),
        CoreError::ServiceNeedsAttention {
            ..
        } => format!("{name} needs the user to sign in again. Ask them to open the Reins app and connect {name}."),
        CoreError::Network {
            ..
        } => format!("The phone could not reach {name}. Try again in a moment."),
        CoreError::GmailNeedsConsent => {
            format!("{name} needs the user's permission again. Ask them to open the Reins app and connect it.")
        }
        other => format!("{name} could not complete the request: {other}"),
    }
}

impl Engine {
    /// The account a call to `service` is about: the one named, or the only one connected. The error is what the AI
    /// is told; it never names accounts the user has not shown.
    pub(crate) fn resolve_service_account(&self, service: &str, named: Option<&str>) -> Result<String, String> {
        let accounts = self.accounts_of(service);
        let name = views::service_name(service);
        match (named, accounts.as_slice()) {
            // Read by the AI, in the activity log and in a failed git push's terminal: so no "the user" or "them".
            (_, []) => Err(format!("{name} is not connected in Reins yet. Add it in the Reins app on the phone: Integrations, {name}.")),
            (Some(wanted), _) => accounts.iter().find(|a| a.eq_ignore_ascii_case(wanted)).cloned().ok_or_else(|| {
                format!("That {name} account is not connected. Call reins_list_accounts with service=\"{service}\" to ask the user to share their accounts, and pick one of those.")
            }),
            (None, [only]) => Ok(only.clone()),
            (None, _) => Err(format!(
                "Several {name} accounts are connected. Call reins_list_accounts with service=\"{service}\" (the user has to allow it) and pass the one you want as `account`."
            )),
        }
    }

    /// Whether the `client_key` of a desktop-only call is the key pinned for its connection when it was paired.
    fn desktop_key_matches(&self, connection: &ConnectionId, call: &ConnectorCall) -> bool {
        let Some(given) = call.str_arg("client_key").and_then(desktop::decode_key) else {
            return false;
        };
        let pinned = match self.store.desktop_key(&connection.0) {
            Ok(pinned) => pinned,
            Err(e) => {
                log::warn!("could not read the desktop app keys: {e}");
                None
            }
        };
        pinned.as_deref().and_then(desktop::decode_key).is_some_and(|k| k == given)
    }

    /// An audit entry for a call to another integration.
    #[allow(clippy::too_many_arguments, reason = "an audit entry is described by this many independent facts")]
    fn audit_connector(
        &self,
        request: &RelayRequest,
        call: &ConnectorCall,
        action: &str,
        outcome: &str,
        detail: &str,
        grant_id: Option<String>,
        items: &[&Item],
    ) -> AuditRecord {
        let mut audit = self.audit(
            request,
            action,
            outcome,
            detail,
            grant_id,
            items.len().max(usize::from(items.is_empty() && outcome != "error" && outcome != "denied")),
            &[],
        );
        audit.service.clone_from(&call.service);
        audit.op.clone_from(&call.op);
        audit.info.query = call.str_arg("query").map(text::one_line);
        audit.info.messages = items.iter().map(|i| audit_message(i)).collect();
        audit.info.targets = items
            .iter()
            .filter(|i| !i.sensitive && !i.resource.is_empty())
            .map(|i| i.resource.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        audit
    }

    async fn fail_connector(
        &self,
        session: &Session,
        request: &RelayRequest,
        call: &ConnectorCall,
        e: &CoreError,
    ) -> Result<(), CoreError> {
        self.fail_connector_text(session, request, call, &ai_message(&call.service, e)).await
    }

    async fn fail_connector_text(
        &self,
        session: &Session,
        request: &RelayRequest,
        call: &ConnectorCall,
        message: &str,
    ) -> Result<(), CoreError> {
        let action = action_of(call);
        let mut audit = self.audit_connector(request, call, action, "error", message, None, &[]);
        audit.count = 1;
        self.finish(
            session,
            request,
            audit,
            RelayOutcome::Error {
                message: message.to_owned(),
            },
        )
        .await
    }

    /// Handles a call to another integration received from the server.
    pub(crate) async fn handle_connector(
        &self,
        session: &Session,
        mut request: RelayRequest,
        call: &ConnectorCall,
    ) -> Result<(), CoreError> {
        let Some(effect) = call.effect() else {
            return self.reject(session, &request, "The request was malformed.").await;
        };
        let desktop_only = call.spec().is_some_and(|s| s.desktop_only);
        // A credential is sealed to the key in the call: it must be the key the user pinned for this connection.
        if desktop_only && !self.desktop_key_matches(&request.connection_id, call) {
            return self.fail_connector_text(session, &request, call, DESKTOP_ONLY).await;
        }
        let Some(connector) = self.connectors.get(&call.service) else {
            return self
                .fail_connector_text(session, &request, call, "That integration is not available on the user's phone.")
                .await;
        };
        let Ok(named) = request.account.as_deref().map(normalize_account_name).transpose() else {
            return self.reject(session, &request, "The account was malformed.").await;
        };
        // Git cannot name an account: with several, the integration picks the one that can do the call.
        let chosen = if desktop_only && named.is_none() && self.accounts_of(&call.service).len() > 1 {
            match connector.choose_account(&self.accounts_of(&call.service), call).await {
                Ok(chosen) => chosen,
                Err(e) => return self.fail_connector(session, &request, call, &e).await,
            }
        } else {
            None
        };
        let resolved = match chosen {
            Some(account) => Ok(account),
            // The desktop app's own questions: the paired app is the account (its key was checked above).
            None if call.service == reins_proto::connector::DESKTOP => Ok(super::desktop::DESKTOP_ACCOUNT.to_owned()),
            None => self.resolve_service_account(&call.service, named.as_deref()),
        };
        request.account = match resolved {
            Ok(account) => Some(account),
            Err(message) => {
                request.account = named;
                return self.fail_connector_text(session, &request, call, &message).await;
            }
        };
        let account = request.account.clone().unwrap_or_default();
        let now = unix_now();

        if effect == Effect::Write {
            // A file the write needs: asked for as an upload, or checked and shown with the preview.
            let Some(file) = self.file_input(session, &request, call).await? else {
                return Ok(());
            };
            let preview = match connector.preview(&account, call).await {
                Ok(p) => file.show(p),
                Err(e) => return self.fail_connector(session, &request, call, &e).await,
            };
            let spec = call.spec();
            let class = preview.class.clone().unwrap_or_else(|| spec.map_or("", |s| s.class).to_owned());
            let once = spec.is_some_and(|s| s.once_only) || preview.once_only;
            // A change that is asked for every time never looks at the permissions.
            let covered = if once {
                BTreeMap::new()
            } else {
                self.store.service_reserve(
                    &request.connection_id,
                    Some(&account),
                    &call.service,
                    "write",
                    &class,
                    std::slice::from_ref(&preview.resource),
                    true,
                    now,
                )?
            };
            if let Some(grant) = covered.get(&preview.resource) {
                return match connector.perform(&account, call).await {
                    Ok(data) => {
                        let detail = format!("done: {}", preview.lines.first().map_or("", String::as_str));
                        let audit = self.audit_connector(
                            &request,
                            call,
                            action_of(call),
                            "sent",
                            &detail,
                            Some(grant.0.clone()),
                            &[],
                        );
                        self.finish(
                            session,
                            &request,
                            audit,
                            RelayOutcome::Result {
                                result: ToolResult::Connector {
                                    data,
                                },
                            },
                        )
                        .await
                    }
                    Err(e) => {
                        // Nothing happened: give the reserved use back.
                        self.store.refund(&BTreeSet::from([grant.clone()]))?;
                        self.fail_connector(session, &request, call, &e).await
                    }
                };
            }
            return self.park_connector(
                &request,
                ParkedConnector {
                    items: Vec::new(),
                    covered: BTreeMap::new(),
                    preview: Some(preview),
                    mcp: None,
                },
            );
        }

        let items = match connector.fetch(&account, call).await {
            Ok(items) => items,
            Err(e) => return self.fail_connector(session, &request, call, &e).await,
        };
        let access = effect.access();
        let resources: Vec<String> =
            items.iter().map(|i| i.resource.clone()).collect::<BTreeSet<_>>().into_iter().collect();
        let any_sensitive = items.iter().any(|i| i.sensitive);
        let covered = self.store.service_reserve(
            &request.connection_id,
            Some(&account),
            &call.service,
            access,
            "",
            &resources,
            !any_sensitive,
            now,
        )?;
        if !any_sensitive && (items.is_empty() || covered.len() == resources.len()) {
            // Content too large to go inline becomes a download link, now that it is released.
            let items = match self.deliver_files(session, connector.as_ref(), &account, &request, items).await {
                Ok(items) => items,
                Err(e) => {
                    self.store.refund(&covered.values().cloned().collect())?;
                    return self.fail_connector(session, &request, call, &e).await;
                }
            };
            let refs: Vec<&Item> = items.iter().collect();
            let grant = covered.values().next().map(|g| g.0.clone());
            let detail = released_detail(call, refs.len());
            let audit = self.audit_connector(&request, call, action_of(call), "released", &detail, grant, &refs);
            let data = items_json(effect, &refs);
            return self
                .finish(
                    session,
                    &request,
                    audit,
                    RelayOutcome::Result {
                        result: ToolResult::Connector {
                            data,
                        },
                    },
                )
                .await;
        }
        let per_item: BTreeMap<String, String> = items
            .iter()
            .filter(|i| !i.sensitive)
            .filter_map(|i| covered.get(&i.resource).map(|g| (i.id.clone(), g.0.clone())))
            .collect();
        self.park_connector(
            &request,
            ParkedConnector {
                items,
                covered: per_item,
                preview: None,
                mcp: None,
            },
        )
    }

    fn park_connector(&self, request: &RelayRequest, held: ParkedConnector) -> Result<(), CoreError> {
        self.park_request(&ParkedRequest {
            request: request.clone(),
            messages: Vec::new(),
            covered: BTreeMap::new(),
            account: request.account.clone(),
            accounts: Vec::new(),
            shared_accounts: Vec::new(),
            connector: Some(held),
        })
    }

    /// The user decided on a parked call to another integration.
    pub(crate) async fn approve_connector(
        &self,
        session: &Session,
        request_id: &str,
        parked: &ParkedRequest,
        choice: &ApprovalChoice,
        now: i64,
    ) -> Result<(), CoreError> {
        let ToolCall::Connector(call) = &parked.request.call else {
            return Err(CoreError::invalid("not a request to another integration"));
        };
        let held = parked.connector.as_ref().ok_or_else(|| CoreError::storage("corrupt parked request"))?;
        let Some(connector) = self.connectors.get(&call.service) else {
            return Err(CoreError::service("that integration is not available"));
        };
        let account =
            parked.request.account.clone().ok_or_else(|| CoreError::invalid("no account for this request"))?;
        let request = &parked.request;
        let effect = call.effect().ok_or_else(|| CoreError::invalid("unknown operation"))?;
        // The app may have been paired again (another key) or removed while this waited.
        if call.spec().is_some_and(|s| s.desktop_only) && !self.desktop_key_matches(&request.connection_id, call) {
            return Err(CoreError::invalid(
                "The desktop app that asked is no longer paired with this key. Deny this request and run git again.",
            ));
        }

        if effect == Effect::Write {
            let preview = held.preview.as_ref().ok_or_else(|| CoreError::storage("corrupt parked request"))?;
            let spec = call.spec();
            if choice.standing.is_some() && (spec.is_some_and(|s| s.once_only) || preview.once_only) {
                return Err(CoreError::invalid("this is asked for every time and cannot be remembered"));
            }
            let known: Vec<(String, String)> =
                std::iter::once((preview.resource.clone(), preview.resource_label.clone()))
                    .chain(preview.parents.iter().cloned())
                    .collect();
            let new_grant = choice
                .standing
                .as_ref()
                .map(|s| {
                    build_service_grant(
                        s,
                        &call.service,
                        "write",
                        preview.class.as_deref().unwrap_or_else(|| spec.map_or("", |s| s.class)),
                        &known,
                        &[],
                        &request.connection_id,
                        &account,
                        now,
                    )
                })
                .transpose()?;
            // The action happens first; if it fails the request stays parked.
            let data = connector.perform(&account, call).await?;
            if let Some(grant) = &new_grant {
                self.store.insert_grant_from(grant, &parked.label(), "approval")?;
            }
            let detail = format!("approved: {}", preview.lines.first().map_or("", String::as_str));
            let mut audit =
                self.audit_connector(request, call, action_of(call), "sent", &detail, new_grant.map(|g| g.id.0), &[]);
            audit.info.targets = crate::quick::approval_targets(parked);
            return self
                .settle(
                    session,
                    request_id,
                    request,
                    audit,
                    RelayOutcome::Result {
                        result: ToolResult::Connector {
                            data,
                        },
                    },
                )
                .await;
        }

        let shown: BTreeSet<&str> = held.items.iter().map(|i| i.id.as_str()).collect();
        if let Some(unknown) = choice.selected_message_ids.iter().find(|id| !shown.contains(id.as_str())) {
            return Err(CoreError::invalid(format!("item {unknown} was not part of this request")));
        }
        let picked: BTreeSet<&str> = choice.selected_message_ids.iter().map(String::as_str).collect();
        let released: Vec<&Item> =
            held.items.iter().filter(|i| held.covered.contains_key(&i.id) || picked.contains(i.id.as_str())).collect();
        if released.is_empty() && !held.items.is_empty() {
            return Err(CoreError::invalid("select at least one item, or deny the request"));
        }
        let access = effect.access();
        let known: Vec<(String, String)> = {
            let mut seen = BTreeSet::new();
            held.items
                .iter()
                .flat_map(|i| {
                    std::iter::once((i.resource.clone(), i.resource_label.clone())).chain(i.parents.iter().cloned())
                })
                .filter(|(id, _)| seen.insert(id.clone()))
                .collect()
        };
        let new_grant = choice
            .standing
            .as_ref()
            .map(|s| {
                build_service_grant(
                    s,
                    &call.service,
                    access,
                    "",
                    &known,
                    &released,
                    &request.connection_id,
                    &account,
                    now,
                )
            })
            .transpose()?;
        let delivered = self
            .deliver_files(
                session,
                connector.as_ref(),
                &account,
                request,
                released.iter().map(|i| (*i).clone()).collect(),
            )
            .await?;
        let data = items_json(effect, &delivered.iter().collect::<Vec<_>>());

        let covering: BTreeSet<GrantId> = held.covered.values().map(|g| GrantId(g.clone())).collect();
        self.store.consume_active(&covering, now)?;
        if let Some(grant) = &new_grant {
            self.store.insert_grant_from(grant, &parked.label(), "approval")?;
        }
        // The AI stopped waiting before the user decided: a read gets a one-time pass so that asking again works.
        let late = request.wait_until.is_some_and(|t| now > t);
        let note = if late && new_grant.is_none() && !released.is_empty() && released.iter().all(|i| !i.sensitive) {
            let resources: Vec<(String, String)> = {
                let mut seen = BTreeSet::new();
                released
                    .iter()
                    .filter(|i| seen.insert(i.resource.clone()))
                    .map(|i| (i.resource.clone(), i.resource_label.clone()))
                    .collect()
            };
            let pass = Grant::new(
                GrantId(uuid::Uuid::new_v4().to_string()),
                request.connection_id.clone(),
                Scope::Service(ServiceScope {
                    service: call.service.clone(),
                    access: access.to_owned(),
                    resources: resources.iter().map(|r| r.0.clone()).collect(),
                    labels: resources.iter().map(|r| r.1.clone()).collect(),
                    any: false,
                    classes: Vec::new(),
                }),
                now,
                Some(now + RETRY_PASS_SECS),
                Some(1),
            )
            .map_err(|e| CoreError::invalid(e.to_string()))?
            .for_account(Some(account.clone()));
            self.store.insert_grant_from(&pass, &parked.label(), "retry")?;
            Some("Approved after the AI stopped waiting. A one-time pass lets it ask again for 15 minutes.".to_owned())
        } else {
            None
        };
        let detail = format!("approved: {}", released_detail(call, released.len()));
        let mut audit = self.audit_connector(
            request,
            call,
            action_of(call),
            "released",
            &detail,
            new_grant.map(|g| g.id.0),
            &released,
        );
        audit.info.note = note;
        self.settle(
            session,
            request_id,
            request,
            audit,
            RelayOutcome::Result {
                result: ToolResult::Connector {
                    data,
                },
            },
        )
        .await
    }

    /// Records the decision, drops the parked request and answers the server.
    async fn settle(
        &self,
        session: &Session,
        request_id: &str,
        request: &RelayRequest,
        audit: AuditRecord,
        outcome: RelayOutcome,
    ) -> Result<(), CoreError> {
        self.store.append_audit(&audit)?;
        self.store.remove_pending(request_id)?;
        self.store.mark_handled(request_id, unix_now())?;
        self.notifier.item_resolved(request_id.to_owned());
        let _ = request;
        self.respond(session, request_id, outcome).await
    }

    /// The audit entry for a refused call to another integration, listing what would have been shared.
    pub(crate) fn denied_connector(&self, parked: &ParkedRequest) -> Option<AuditRecord> {
        let ToolCall::Connector(call) = &parked.request.call else {
            return None;
        };
        let held = parked.connector.as_ref()?;
        let items: Vec<&Item> = held.items.iter().collect();
        let mut audit = self.audit_connector(
            &parked.request,
            call,
            action_of(call),
            "denied",
            &crate::autopilot::context::denied_detail(),
            None,
            &items,
        );
        audit.count = parked.count();
        Some(audit)
    }
}

use reins_proto::gmail::ToolCall;

/// What the activity log calls the action: "list", "read", "search", "send" or "write".
pub(crate) fn action_of(call: &ConnectorCall) -> &'static str {
    match call.effect() {
        Some(Effect::List) => "list",
        Some(Effect::Search) => "search",
        Some(Effect::Write) if call.op == "send" => "send",
        Some(Effect::Write) => "write",
        _ => "read",
    }
}

fn released_detail(call: &ConnectorCall, n: usize) -> String {
    let what = call.spec().map_or(call.op.as_str(), |s| s.title);
    format!(
        "{what}: {n} item{}",
        if n == 1 {
            ""
        } else {
            "s"
        }
    )
}

fn audit_message(item: &Item) -> AuditMessage {
    AuditMessage {
        id: item.id.clone(),
        text: item.text(),
        from: text::one_line(&item.from),
        subject: text::one_line(&item.title),
        date: item.date,
    }
}

/// Builds the standing permission the user asked for alongside an approval on another integration. Pure and strict.
#[allow(clippy::too_many_arguments, reason = "a grant is described by this many independent facts")]
pub fn build_service_grant(
    standing: &StandingGrant,
    service: &str,
    access: &str,
    class: &str,
    known: &[(String, String)],
    released: &[&Item],
    connection: &ConnectionId,
    account: &str,
    now: i64,
) -> Result<Grant, CoreError> {
    if !released.is_empty() && released.iter().all(|i| i.sensitive) {
        return Err(CoreError::invalid("this cannot be remembered; it is asked for every time"));
    }
    let expires_at = match standing.duration_secs {
        None => None,
        Some(0) => return Err(CoreError::invalid("the grant duration must be longer than zero")),
        Some(secs) => Some(now.saturating_add(i64::try_from(secs).unwrap_or(i64::MAX))),
    };
    let s = &standing.scope;
    let scope = if s.all_mail {
        if access == "write" {
            return Err(CoreError::invalid("writing cannot be allowed everywhere"));
        }
        if !s.resources.is_empty() {
            return Err(CoreError::invalid("choose everything, or name what it covers, not both"));
        }
        ServiceScope {
            service: service.to_owned(),
            access: access.to_owned(),
            resources: Vec::new(),
            labels: Vec::new(),
            any: true,
            classes: Vec::new(),
        }
    } else {
        let mut resources = Vec::new();
        let mut labels = Vec::new();
        for id in &s.resources {
            let Some((_, label)) = known.iter().find(|(k, _)| k == id) else {
                return Err(CoreError::invalid(format!("{id} was not part of this request")));
            };
            if !resources.contains(id) {
                resources.push(id.clone());
                labels.push(text::one_line(label));
            }
        }
        if resources.is_empty() {
            return Err(CoreError::invalid("choose what the permission covers"));
        }
        // A permission to change things allows the kinds of change the user named; none named means the kind of this
        // request, never every kind.
        let known_classes = reins_proto::connector::classes(service);
        let mut classes: Vec<String> = Vec::new();
        if access == "write" {
            for c in &s.classes {
                if !known_classes.iter().any(|k| k.id == c) {
                    return Err(CoreError::invalid(format!("{c} is not a kind of change here")));
                }
                if !classes.contains(c) {
                    classes.push(c.clone());
                }
            }
            if classes.is_empty() && !class.is_empty() {
                classes.push(class.to_owned());
            }
        } else if !s.classes.is_empty() {
            return Err(CoreError::invalid("only a permission to change things names kinds of change"));
        }
        ServiceScope {
            service: service.to_owned(),
            access: access.to_owned(),
            resources,
            labels,
            any: false,
            classes,
        }
    };
    Grant::new(
        GrantId(uuid::Uuid::new_v4().to_string()),
        connection.clone(),
        Scope::Service(scope),
        now,
        expires_at,
        standing.max_uses,
    )
    .map(|g| g.for_account(Some(account.to_owned())))
    .map_err(|e| CoreError::invalid(e.to_string()))
}
