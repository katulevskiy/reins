//! Proposals (design §3): a viewer adds a connection or a credential as a proposal; the primary keeps it (the records
//! are written as its own) or discards it.
//!
//! The viewer's display fields (`kind`, `title`) travel under the view key and are never what the primary decides
//! on: what it adds is sealed to the credential key, so only the primary opens it, and Keep / Discard is shown from
//! [`Engine::proposal_preview`], the rows themselves. Keep then takes the digest of what was previewed (the proposing
//! phone cannot change it after the user looked), adds only what is not there yet, keeps only the columns a proposal
//! may set (what a server says of itself, such as its tools and their read-only marks, is listed again by the primary),
//! takes tokens of integrations only (never a key of Reins, of payments or of this installation), and checks that the
//! rows are what the proposal says: an account with its token, or an MCP server with its own.

use data_encoding::BASE64URL_NOPAD;
use reins_proto::canon::{hex32, sha256};
use reins_proto::oplog::{Change, MAX_PROPOSAL_BODY_BYTES, OpClass, Table};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json, json};

use super::{Keys, invalid, table_name};
use crate::CoreError;
use crate::engine::Engine;
use crate::roles::Standing;
use crate::roles::keys::seal_to;
use crate::store::oplog;
use crate::store::records;
use crate::store::unix_now;

/// The tables a proposal may add to: an account, its token, an MCP server. Never a grant, a mode, a paired computer,
/// a setting, or how an existing connection looks.
const PROPOSABLE: &[Table] = &[Table::Accounts, Table::Secrets, Table::McpServers];
/// An MCP server's id: what the app makes (lowercase letters, digits, `-`), at most this long.
const MAX_MCP_ID: usize = 64;
/// The integrations whose accounts and tokens a viewer may propose.
const INTEGRATIONS: &[&str] = &["github", "gitlab", "codeberg", "bitbucket", "telegram"];
/// The tokens of MCP servers (their account is the server's id).
const MCP_TOKENS: &str = crate::store::MCP_SECRET_SERVICE;
/// A proposal's kind for an MCP server.
const KIND_MCP: &str = "mcp";

/// A proposal waiting for the primary.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ProposalView {
    /// `<author>/<id>`: pass it back to preview, keep, discard or withdraw.
    pub id: String,
    pub author_id: String,
    /// The proposing phone's name in the roster.
    pub author_name: String,
    /// What it adds ("github", "mcp", …) and how the proposing phone named it: display only, never what Keep shows.
    pub kind: String,
    pub title: String,
    pub received_at: i64,
    /// This phone proposed it.
    pub mine: bool,
}

/// One row a proposal adds, as the primary opened it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ProposalRowView {
    /// `accounts`, `secrets` or `mcp_servers`.
    pub table: String,
    pub key: String,
    /// What the row is: the service and account, or the server's name and URL (never a token).
    pub summary: String,
    /// A row with this key exists already: Keep refuses the proposal.
    pub replaces: bool,
}

/// What a proposal really adds (opened with the credential key): Keep / Discard is shown from this.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ProposalPreview {
    pub id: String,
    pub author_name: String,
    pub kind: String,
    pub rows: Vec<ProposalRowView>,
    /// Hand it back to `proposal_keep`: Keep refuses anything else than what was shown.
    pub digest: String,
    /// Why Keep would refuse it, if it would.
    pub refused: Option<String>,
}

/// One row a proposal adds (a record of a [`PROPOSABLE`] table, as [`records`] writes it).
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ProposedRow {
    table: Table,
    value: Json,
}

/// The columns a proposal may set, per table (everything else is the primary's own, or derived by it).
fn allowed_columns(table: Table) -> &'static [&'static str] {
    match table {
        Table::Accounts => &["service", "account"],
        Table::Secrets => &["service", "account", "value"],
        // Never `client`: how the primary signs in to the server (its OAuth registration) is the primary's own, or an
        // attacker's token endpoint would receive the authorization code.
        Table::McpServers => &["id", "name", "url", "auth"],
        _ => &[],
    }
}

fn text<'a>(row: &'a Json, column: &str) -> &'a str {
    row.get(column).and_then(Json::as_str).unwrap_or_default()
}

/// The rows as the primary writes them: the allowed columns only, what the primary derives itself set afresh.
fn sanitized(rows: &[ProposedRow], now: i64) -> Result<Vec<ProposedRow>, String> {
    rows.iter()
        .map(|r| {
            let allowed = allowed_columns(r.table);
            let source = r.value.as_object().ok_or("a row is not an object")?;
            if let Some(extra) = source.keys().find(|k| !allowed.contains(&k.as_str())) {
                return Err(format!("a proposal may not set {extra}"));
            }
            let mut value: Map<String, Json> = source.clone();
            for column in allowed {
                match value.get(*column) {
                    Some(v) if v.is_string() || (r.table == Table::Secrets && *column == "value") => {}
                    _ => return Err(format!("a proposal's row lacks {column}")),
                }
            }
            match r.table {
                Table::Accounts => {
                    value.insert("added_at".to_owned(), Json::from(now));
                }
                Table::McpServers => {
                    let id = text(&r.value, "id");
                    let well_formed = !id.is_empty()
                        && id.len() <= MAX_MCP_ID
                        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
                    if !well_formed {
                        return Err("an MCP server's id is lowercase letters, digits and -".to_owned());
                    }
                    // What the server says of itself (its tools, which are heavy or read-only, its status) is listed
                    // again by the primary, never taken from the proposing phone.
                    for (column, default) in
                        [("tools", json!("[]")), ("heavy", json!("[]")), ("status", json!("ok")), ("error", Json::Null)]
                    {
                        value.insert(column.to_owned(), default);
                    }
                    value.insert("listed_at".to_owned(), Json::Null);
                    value.insert("added_at".to_owned(), Json::from(now));
                    // The primary finds and registers how to sign in itself.
                    value.insert("client".to_owned(), Json::Null);
                    let url = crate::mcp::checked_url(text(&r.value, "url")).map_err(|e| e.to_string())?;
                    if url.scheme() != "https" {
                        return Err("an MCP server is reached over https only".to_owned());
                    }
                    if !matches!(text(&r.value, "auth"), "none" | "token") {
                        return Err("an MCP server signs in with no key or a token only".to_owned());
                    }
                }
                _ => {}
            }
            Ok(ProposedRow {
                table: r.table,
                value: Json::Object(value),
            })
        })
        .collect()
}

/// Whether the rows are what a proposal of `kind` adds: an account of that integration with (or without) its token,
/// or an MCP server with (or without) its own token. Tokens of anything else are refused.
fn check_kind(kind: &str, rows: &[ProposedRow]) -> Result<(), String> {
    let of = |t: Table| rows.iter().filter(move |r| r.table == t);
    for secret in of(Table::Secrets) {
        let (service, account) = (text(&secret.value, "service"), text(&secret.value, "account"));
        let ok = if service == MCP_TOKENS {
            kind == KIND_MCP && of(Table::McpServers).any(|m| text(&m.value, "id") == account)
        } else {
            INTEGRATIONS.contains(&service)
                && kind == service
                && of(Table::Accounts)
                    .any(|a| text(&a.value, "service") == service && text(&a.value, "account") == account)
        };
        if !ok {
            return Err(format!("a token for {service} does not belong to what this proposal adds"));
        }
    }
    match kind {
        KIND_MCP => {
            if of(Table::McpServers).count() != 1 || of(Table::Accounts).next().is_some() {
                return Err("an MCP server proposal adds one server and its token".to_owned());
            }
        }
        service if INTEGRATIONS.contains(&service) => {
            let accounts: Vec<_> = of(Table::Accounts).collect();
            if accounts.len() != 1
                || text(&accounts[0].value, "service") != service
                || of(Table::McpServers).next().is_some()
            {
                return Err(format!("a {service} proposal adds one {service} account and its token"));
            }
        }
        other => return Err(format!("a proposal cannot add {other}")),
    }
    Ok(())
}

/// What Keep / Discard was shown: the kind and title as they were, and the payload's exact bytes.
fn proposal_digest(kind: &str, title: &str, plain: &[u8]) -> String {
    let mut c = reins_proto::canon::Canon::new("reins/proposal-preview/v1");
    c.str(kind).str(title).key(&sha256(plain));
    hex32(&sha256(&c.finish()))
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
        check_kind(kind, &rows).map_err(CoreError::invalid)?;
        sanitized(&rows, unix_now()).map_err(CoreError::invalid)?;
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
        if let Err(e) = self.upload_outbox(&session, &account).await {
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

    /// The rows a waiting proposal adds, opened with the credential key, and their digest (on the primary).
    fn opened_proposal(
        &self,
        account: &crate::roles::Account,
        id: &str,
    ) -> Result<(oplog::Proposal, Vec<ProposedRow>, String), CoreError> {
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
        let field = |name: &str| proposal.value.get(name).and_then(Json::as_str).unwrap_or_default().to_owned();
        let digest = proposal_digest(&field("kind"), &field("title"), &plain);
        Ok((proposal, rows, digest))
    }

    /// Why Keep would refuse these rows: a table or column a proposal may not set, rows that are not what the kind
    /// says, a token that is not an integration's, or a row that exists already (a proposal adds, never replaces).
    fn keep_refusal(kind: &str, rows: &[ProposedRow], exists: &[bool]) -> Option<String> {
        if rows.iter().any(|r| !PROPOSABLE.contains(&r.table)) {
            return Some("The proposal adds something a proposal may not.".to_owned());
        }
        if let Err(why) = check_kind(kind, rows).and_then(|()| sanitized(rows, 0).map(drop)) {
            return Some(why);
        }
        exists.iter().any(|e| *e).then(|| "The proposal would replace something this phone has.".to_owned())
    }

    /// Which of `rows` exist already (by their key in the store).
    fn existing(&self, rows: &[ProposedRow]) -> Result<Vec<bool>, CoreError> {
        let have: std::collections::HashSet<(Table, String)> =
            self.store.sync_rows()?.into_iter().map(|r| (r.table, r.key)).collect();
        Ok(rows
            .iter()
            .map(|r| records::key_of_row(r.table, &r.value).is_none_or(|k| have.contains(&(r.table, k))))
            .collect())
    }

    /// On the primary: what a proposal really adds, for Keep / Discard (never its display fields).
    pub async fn proposal_preview(&self, id: &str) -> Result<ProposalPreview, CoreError> {
        let session = self.session()?;
        self.require_decider()?;
        let account = self.roles_account(&session).await?;
        let keys = self.primary_keys(&account)?;
        let (proposal, rows, digest) = self.opened_proposal(&account, id)?;
        let kind = proposal.value.get("kind").and_then(Json::as_str).unwrap_or_default().to_owned();
        let exists = self.existing(&rows)?;
        let views = rows
            .iter()
            .zip(&exists)
            .map(|(r, replaces)| ProposalRowView {
                table: records::sql_table(r.table).unwrap_or_default().to_owned(),
                key: records::key_of_row(r.table, &r.value).unwrap_or_default(),
                summary: match r.table {
                    Table::McpServers => format!("{} · {}", text(&r.value, "name"), text(&r.value, "url")),
                    _ => format!("{} · {}", text(&r.value, "service"), text(&r.value, "account")),
                },
                replaces: *replaces,
            })
            .collect();
        Ok(ProposalPreview {
            id: id.to_owned(),
            author_name: keys
                .roster
                .current()
                .member(&proposal.author)
                .map_or_else(|| proposal.author.clone(), |m| m.name.clone()),
            refused: Self::keep_refusal(&kind, &rows, &exists),
            kind,
            rows: views,
            digest,
        })
    }

    /// On the primary: adds what the proposal previewed as `digest` (and only that) as this phone's own records, then
    /// resolves it. Refused if the proposal changed since, or [`ProposalPreview::refused`] says so.
    pub async fn proposal_keep(&self, id: &str, digest: &str) -> Result<(), CoreError> {
        let session = self.session()?;
        self.require_decider_fresh(&session).await?;
        let account = self.roles_account(&session).await?;
        let keys = self.primary_keys(&account)?;
        let (proposal, rows, opened) = self.opened_proposal(&account, id)?;
        if opened != digest {
            return Err(CoreError::invalid("The proposal changed since it was shown. Look at it again."));
        }
        let kind = proposal.value.get("kind").and_then(Json::as_str).unwrap_or_default().to_owned();
        let exists = self.existing(&rows)?;
        if let Some(why) = Self::keep_refusal(&kind, &rows, &exists) {
            return Err(CoreError::invalid(why));
        }
        // Every row checked before any is written, and all written in one go (a bad one leaves none behind).
        let rows = sanitized(&rows, unix_now()).map_err(CoreError::invalid)?;
        let rows: Vec<(Table, Json)> = rows.into_iter().map(|r| (r.table, r.value)).collect();
        self.store.insert_sync_rows(&rows)?;
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
        if let Err(e) = self.upload_outbox(session, account).await {
            log::warn!("resolving the proposal waits to reach the server: {e}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn row(table: Table, value: Json) -> ProposedRow {
        ProposedRow {
            table,
            value,
        }
    }

    #[test]
    fn what_keep_shows_is_bound_to_the_kind_the_title_and_the_payload() {
        let base = proposal_digest("github", "GitHub · octo", b"rows");
        assert_ne!(base, proposal_digest("gitlab", "GitHub · octo", b"rows"));
        assert_ne!(base, proposal_digest("github", "GitHub · octo2", b"rows"));
        assert_ne!(base, proposal_digest("github", "GitHub · octo", b"rows2"));
        assert_eq!(base, proposal_digest("github", "GitHub · octo", b"rows"));
    }

    #[test]
    fn an_mcp_server_proposal_sets_no_client_and_signs_in_only_as_the_primary_decides() {
        let server = |extra: Json| {
            let mut v = json!({"id": "m1", "name": "N", "url": "https://notes.example/mcp", "auth": "token"});
            for (k, x) in extra.as_object().unwrap() {
                v[k] = x.clone();
            }
            vec![row(Table::McpServers, v)]
        };
        let kept = sanitized(&server(json!({})), 7).unwrap();
        assert_eq!(kept[0].value["client"], Json::Null);
        assert_eq!(kept[0].value["tools"], json!("[]"));
        for (bad, why) in [
            (json!({"id": "M1"}), "an id the app does not make"),
            (json!({"id": "x".repeat(65)}), "an id too long"),
            (json!({"id": "a/../b"}), "a path in the id"),
            (json!({"client": "{\"token_endpoint\":\"https://evil.example/token\"}"}), "a client"),
            (json!({"auth": "oauth"}), "OAuth"),
            (json!({"url": "https://real.example@evil.example/"}), "a user name in the URL"),
            (json!({"url": "http://notes.example/"}), "plain http"),
            (json!({"url": "https://notes.example/#frag"}), "a fragment"),
        ] {
            assert!(sanitized(&server(bad), 7).is_err(), "{why}");
        }
    }
}

#[cfg(test)]
mod icon_tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_viewer_cannot_restyle_a_connection() {
        let icon = vec![ProposedRow {
            table: Table::ConnectionPrefs,
            value: json!({"connection_id": "c1", "icon": "star"}),
        }];
        assert!(check_kind("icon", &icon).is_err());
        assert!(!PROPOSABLE.contains(&Table::ConnectionPrefs));
    }
}
