//! Proposals (design §3): a viewer adds a connection or a credential as a proposal; the primary keeps it (the records
//! are written as its own) or discards it. The viewer's display fields travel under the view key; what it adds (a
//! token, an MCP server) is sealed to the credential key, so only the primary opens it and renders Keep / Discard from
//! that, never from the display fields.

use data_encoding::BASE64URL_NOPAD;
use reins_proto::oplog::{Change, MAX_PROPOSAL_BODY_BYTES, OpClass, Table};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

use super::{Keys, invalid, table_name};
use crate::CoreError;
use crate::engine::Engine;
use crate::roles::Standing;
use crate::roles::keys::seal_to;
use crate::store::oplog;
use crate::store::unix_now;

/// The tables a proposal may add to: an account, its token, an MCP server, a connection's icon. Never a grant, a
/// mode, a paired computer or a setting.
const PROPOSABLE: &[Table] = &[Table::Accounts, Table::Secrets, Table::McpServers, Table::ConnectionPrefs];

/// A proposal waiting for the primary.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ProposalView {
    /// `<author>/<id>`: pass it back to keep, discard or withdraw.
    pub id: String,
    pub author_id: String,
    /// The proposing phone's name in the roster.
    pub author_name: String,
    /// What it adds ("github", "mcp", …) and how the proposing phone named it: display only.
    pub kind: String,
    pub title: String,
    pub received_at: i64,
    /// This phone proposed it.
    pub mine: bool,
}

/// One row a proposal adds (a record of a [`PROPOSABLE`] table, as [`records`] writes it).
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ProposedRow {
    table: Table,
    value: Json,
}

impl Engine {
    /// On a viewer: proposes `rows` (a JSON list of `{"table", "value"}`, the tables an account, its token, an MCP
    /// server or a connection's icon live in) for the primary to keep or discard. Returns the proposal's id.
    pub async fn proposal_create(&self, kind: &str, title: &str, rows_json: &str) -> Result<String, CoreError> {
        let rows: Vec<ProposedRow> =
            serde_json::from_str(rows_json).map_err(|_| CoreError::invalid("A proposal is a list of rows."))?;
        if rows.is_empty() || rows.iter().any(|r| !PROPOSABLE.contains(&r.table) || !r.value.is_object()) {
            return Err(CoreError::invalid("A proposal adds an account, a token, an MCP server or an icon only."));
        }
        let session = self.session()?;
        let account = self.roles_account(&session).await?;
        let keys = self.sync_keys(&account)?.ok_or_else(|| CoreError::invalid("This account has no phone roles."))?;
        if keys.standing != Standing::Viewer {
            return Err(CoreError::invalid("Only a viewer proposes; the primary adds it itself."));
        }
        let sealed = seal_to(&keys.roster.current().credential_pub, &serde_json::to_vec(&rows).map_err(invalid)?)?;
        let id = format!("{}/{}", keys.me, BASE64URL_NOPAD.encode(&crate::crypto::random_bytes::<12>()?));
        let value = json!({
            "kind": kind.chars().take(64).collect::<String>(),
            "title": title.chars().take(200).collect::<String>(),
            "payload": BASE64URL_NOPAD.encode(&sealed),
        });
        if value.to_string().len() > MAX_PROPOSAL_BODY_BYTES / 2 {
            return Err(CoreError::invalid("This proposal is too large."));
        }
        let change = Change::Put {
            table: Table::Proposals,
            key: id.clone(),
            value,
        };
        let _guard = self.account_sync.lock().await;
        self.enqueue(&account, &keys, &keys.ring, OpClass::Proposal, change, (&table_name(Table::Proposals), &id))?;
        if let Err(e) = self.upload_outbox(&session).await {
            log::warn!("the proposal waits to reach the server: {e}");
        }
        Ok(id)
    }

    /// On the viewer that proposed it: takes a proposal back.
    pub async fn proposal_withdraw(&self, id: &str) -> Result<(), CoreError> {
        let session = self.session()?;
        let account = self.roles_account(&session).await?;
        let keys = self.sync_keys(&account)?.ok_or_else(|| CoreError::invalid("This account has no phone roles."))?;
        if !id.strip_prefix(keys.me.as_str()).is_some_and(|rest| rest.starts_with('/')) {
            return Err(CoreError::invalid("Only the phone that proposed it takes it back."));
        }
        self.resolve(&session, &account, &keys, id, OpClass::Proposal).await
    }

    /// The proposals waiting for the primary, oldest first.
    pub async fn proposals(&self) -> Result<Vec<ProposalView>, CoreError> {
        let session = self.session()?;
        let account = self.roles_account(&session).await?;
        let Some(keys) = self.sync_keys(&account)? else {
            return Ok(Vec::new());
        };
        let waiting = self.store.oplog(|c| oplog::proposals(c, unix_now()))??;
        let current = keys.roster.current();
        Ok(waiting
            .into_iter()
            .map(|p| ProposalView {
                author_name: current.member(&p.author).map_or_else(|| p.author.clone(), |m| m.name.clone()),
                kind: p.value.get("kind").and_then(Json::as_str).unwrap_or_default().to_owned(),
                title: p.value.get("title").and_then(Json::as_str).unwrap_or_default().to_owned(),
                mine: p.author == keys.me,
                id: p.key,
                author_id: p.author,
                received_at: p.received_at,
            })
            .collect())
    }

    /// On the primary: writes what a proposal adds as this phone's own records (they go out with the next publish),
    /// then resolves it. What is written is what the payload holds, opened with the credential key.
    pub async fn proposal_keep(&self, id: &str) -> Result<(), CoreError> {
        let session = self.session()?;
        self.require_decider_fresh(&session).await?;
        let account = self.roles_account(&session).await?;
        let keys = self.primary_keys(&account)?;
        let waiting = self.store.oplog(|c| oplog::proposals(c, unix_now()))??;
        let proposal = waiting.into_iter().find(|p| p.key == id).ok_or(CoreError::NotFound)?;
        let seed =
            self.owner_seed(&account.slot)?.ok_or_else(|| CoreError::invalid("This phone holds no owner key."))?;
        let raw = proposal
            .value
            .get("payload")
            .and_then(Json::as_str)
            .and_then(|p| BASE64URL_NOPAD.decode(p.as_bytes()).ok())
            .ok_or_else(|| CoreError::invalid("The proposal is malformed."))?;
        let plain = seed.credential_key().open(&raw).map_err(|_| CoreError::invalid("The proposal does not open."))?;
        let rows: Vec<ProposedRow> =
            serde_json::from_slice(&plain).map_err(|_| CoreError::invalid("The proposal is malformed."))?;
        if rows.iter().any(|r| !PROPOSABLE.contains(&r.table)) {
            return Err(CoreError::invalid("The proposal adds something a proposal may not."));
        }
        for row in &rows {
            self.store.write_sync_row(row.table, "proposal", None, Some(&row.value))?;
        }
        self.resolve(&session, &account, &keys, id, OpClass::Primary).await?;
        self.live_sync_now().await
    }

    /// On the primary: drops a proposal unopened.
    pub async fn proposal_discard(&self, id: &str) -> Result<(), CoreError> {
        let session = self.session()?;
        self.require_decider()?;
        let account = self.roles_account(&session).await?;
        let keys = self.primary_keys(&account)?;
        self.resolve(&session, &account, &keys, id, OpClass::Primary).await
    }

    fn primary_keys(&self, account: &crate::roles::Account) -> Result<Keys, CoreError> {
        let keys = self.sync_keys(account)?.ok_or_else(|| CoreError::invalid("This account has no phone roles."))?;
        if keys.standing != Standing::Primary {
            return Err(CoreError::ViewerOnly);
        }
        Ok(keys)
    }

    async fn resolve(
        &self,
        session: &crate::session::Session,
        account: &crate::roles::Account,
        keys: &Keys,
        id: &str,
        class: OpClass,
    ) -> Result<(), CoreError> {
        let change = Change::Remove {
            table: Table::Proposals,
            key: id.to_owned(),
        };
        let _guard = self.account_sync.lock().await;
        self.enqueue(account, keys, &keys.ring, class, change, (&table_name(Table::Proposals), id))?;
        if let Err(e) = self.upload_outbox(session).await {
            log::warn!("resolving the proposal waits to reach the server: {e}");
        }
        Ok(())
    }
}
