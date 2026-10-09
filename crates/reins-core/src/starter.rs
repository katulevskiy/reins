//! The starting policy: what a newly connected AI may do before it has asked for anything. Chosen in one screen during
//! setup and changed under Grants. "Reads for a day" gives the new connection ordinary grants (listed under Grants,
//! revocable, logged) to search and read each connected integration for 24 hours; everything else asks as usual:
//! sending, changing or deleting anything, the vault, the desktop app's questions and secrets, and the hard floor.
//! Nothing that looks like a code or a password is ever released by a grant, these included.

use reins_policy::{Grant, ReadScope, Scope, ServiceScope};
use reins_proto::ids::{ConnectionId, GrantId};

use crate::CoreError;
use crate::engine::Engine;
use crate::store::{AuditInfo, AuditRecord};
use crate::types::StartingPolicy;
use crate::views;

/// How long the reads of a new connection are allowed.
pub const STARTER_SECS: i64 = 86_400;
/// The grants' origin, as `GrantView.origin` shows it.
pub const ORIGIN: &str = "starter";
const META_KEY: &str = "starting_policy";
/// Integrations a starting grant never covers: the vault (names of logins, secrets), the desktop app (its questions,
/// secrets and SSH sign-ins are asked every time anyway) and files uploaded for one operation.
const NEVER: [&str; 3] = ["vault", reins_proto::connector::DESKTOP, crate::blob::SERVICE_FILES];

impl Engine {
    /// What a new connection may do at first; `None` until the user chose (setup or Grants), which acts as "ask every
    /// time".
    pub fn starting_policy(&self) -> Result<Option<StartingPolicy>, CoreError> {
        Ok(match self.store.meta_get(META_KEY)?.as_deref() {
            Some("reads_for_a_day") => Some(StartingPolicy::ReadsForADay),
            Some("ask_every_time") => Some(StartingPolicy::AskEveryTime),
            _ => None,
        })
    }

    /// Applies to connections made from now on; grants already given stay until they end or are revoked.
    pub fn set_starting_policy(&self, policy: StartingPolicy) -> Result<(), CoreError> {
        self.store.meta_set(
            META_KEY,
            match policy {
                StartingPolicy::AskEveryTime => "ask_every_time",
                StartingPolicy::ReadsForADay => "reads_for_a_day",
            },
        )
    }

    /// The grants "reads for a day" gives a connection the user just approved: one per connected integration (every
    /// account of it) and per added MCP server (its read-only tools), each for [`STARTER_SECS`]. Returns how many.
    pub(crate) fn grant_starter_reads(&self, connection: &str, label: &str, now: i64) -> Result<usize, CoreError> {
        let connection_id = ConnectionId(connection.to_owned());
        let expires = Some(now + STARTER_SECS);
        let grant = |scope: Scope| {
            Grant::new(GrantId(uuid::Uuid::new_v4().to_string()), connection_id.clone(), scope, now, expires, None)
                .map_err(|e| CoreError::invalid(e.to_string()))
        };
        let service = |service: String| {
            Scope::Service(ServiceScope {
                service,
                access: "read".to_owned(),
                resources: Vec::new(),
                labels: Vec::new(),
                any: true,
                classes: Vec::new(),
                ops: Vec::new(),
            })
        };
        let mut grants = Vec::new();
        for integration in self.integrations() {
            if NEVER.contains(&integration.as_str()) {
                continue;
            }
            grants.push(if integration == views::SERVICE_GMAIL {
                grant(Scope::Read(ReadScope {
                    any: true,
                    ..ReadScope::default()
                }))?
            } else {
                grant(service(integration))?
            });
        }
        for server in self.store.mcp_servers()? {
            if server.tools.iter().any(|t| t.read_only) {
                grants.push(grant(service(crate::mcp::grant_service(&server.id)))?);
            }
        }
        for g in &grants {
            self.store.insert_grant_from(g, label, ORIGIN)?;
        }
        if !grants.is_empty() {
            let summary = format!("reading for 24 hours ({})", plural(grants.len()));
            self.store.append_audit(&AuditRecord {
                seq: 0,
                at: now,
                connection_id: connection.to_owned(),
                connection_label: label.to_owned(),
                action: "grant".to_owned(),
                outcome: "granted".to_owned(),
                detail: format!("allowed: {summary}"),
                grant_id: grants.first().map(|g| g.id.0.clone()),
                service: String::new(),
                account: None,
                count: u32::try_from(grants.len()).unwrap_or(u32::MAX),
                op: String::new(),
                info: AuditInfo {
                    grant_summary: Some(summary),
                    note: Some(
                        "From your starting rule: a new AI may search and read for a day. Sending, changing and \
                         anything that looks like a code or a password still ask. Change the rule under Grants."
                            .to_owned(),
                    ),
                    ..AuditInfo::default()
                },
            })?;
        }
        Ok(grants.len())
    }
}

fn plural(n: usize) -> String {
    if n == 1 {
        "1 integration".to_owned()
    } else {
        format!("{n} integrations")
    }
}
