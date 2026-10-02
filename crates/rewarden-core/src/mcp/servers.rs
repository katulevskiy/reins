//! Adding, signing in to, refreshing and removing MCP servers, and running one request on a server with a working
//! session and token (renewed once after a 401, the session reopened once after it ended).

use std::collections::HashMap;

use rewarden_proto::ids::ConnectionId;
use rewarden_proto::remote_mcp::{MAX_SERVERS, McpServerReport, ProxyCall};
use serde_json::{Map, Value, json};
use url::Url;
use zeroize::Zeroizing;

use super::client::{Endpoint, McpError, result_of};
use super::oauth::{self, OAuthClient, PendingSignIn, RefreshError};
use super::{
    Connected, Credentials, McpAddStep, McpServerView, REDIRECT_URI, checked_url, display_name, grant_service,
    new_server_id, server_api, server_view,
};
use crate::engine::Engine;
use crate::session::Session;
use crate::store::{MCP_SECRET_SERVICE, MCP_SIGN_IN_SERVICE, McpTool, StoredMcpServer, unix_now};
use crate::types::PendingKind;
use crate::views::ParkedRequest;
use crate::{CoreError, text};

/// Access tokens are renewed this long before they expire.
const EXPIRY_MARGIN_SECS: i64 = 60;
const MAX_TOKEN_LEN: usize = 8_192;

/// One request to a server.
pub enum Op<'a> {
    ListTools,
    Call {
        tool: &'a str,
        arguments: &'a Map<String, Value>,
    },
    /// A call the Rewarden server makes for the phone (a heavy tool).
    Proxy {
        tool: &'a str,
        arguments: &'a Map<String, Value>,
        connection_id: &'a ConnectionId,
        session: &'a Session,
    },
}

pub enum Output {
    Tools(Vec<McpTool>),
    Result(Value),
}

/// Why a request to a server failed.
#[derive(Debug)]
pub enum Failure {
    /// No usable token: the user has to sign in again (or give a new token). `challenge` is the server's
    /// `WWW-Authenticate`, when it answered 401.
    NeedsSignIn {
        challenge: Option<String>,
    },
    Mcp(McpError),
}

impl From<CoreError> for Failure {
    fn from(e: CoreError) -> Self {
        Self::Mcp(McpError::Core(e))
    }
}

impl Failure {
    /// What the app shows.
    pub fn into_core(self, server_name: &str) -> CoreError {
        match self {
            Self::NeedsSignIn {
                ..
            } => CoreError::needs_attention(format!("Sign in to {server_name} again under MCP servers.")),
            Self::Mcp(McpError::Core(e)) => e,
            Self::Mcp(e) => CoreError::service(format!("{server_name}: {}", e.describe())),
        }
    }
}

fn client_of(server: &StoredMcpServer) -> Option<OAuthClient> {
    server.client.as_deref().and_then(|c| serde_json::from_str(c).ok())
}

fn same_address(a: &str, b: &Url) -> bool {
    Url::parse(a).is_ok_and(|a| a.as_str().trim_end_matches('/') == b.as_str().trim_end_matches('/'))
}

fn connect_error(e: &McpError) -> CoreError {
    match e {
        McpError::Core(core) => core.clone(),
        other => CoreError::service(format!("Could not use the server: {}.", other.describe())),
    }
}

impl Engine {
    // ---- secrets -----------------------------------------------------------------------------------------------

    fn mcp_credentials(&self, id: &str) -> Result<Option<Credentials>, CoreError> {
        let Some(raw) = self.store.secret_get(MCP_SECRET_SERVICE, id)? else {
            return Ok(None);
        };
        let raw = Zeroizing::new(raw);
        Ok(serde_json::from_slice(&raw).ok())
    }

    fn mcp_save_credentials(&self, id: &str, creds: &Credentials) -> Result<(), CoreError> {
        let raw = Zeroizing::new(serde_json::to_vec(creds).map_err(|e| CoreError::storage(e.to_string()))?);
        self.store.secret_put(MCP_SECRET_SERVICE, id, &raw)
    }

    async fn mcp_forget_session(&self, id: &str) {
        self.mcp.sessions.lock().await.remove(id);
    }

    // ---- tokens and sessions -----------------------------------------------------------------------------------

    /// The token to send now (renewed first when it is about to expire); `None` for a server without sign-in.
    async fn mcp_token(&self, server: &StoredMcpServer) -> Result<Option<Zeroizing<String>>, Failure> {
        if server.auth == "none" {
            return Ok(None);
        }
        let creds = self.mcp_credentials(&server.id)?;
        let Some(access) = creds.as_ref().and_then(|c| c.access_token.clone()) else {
            return Err(Failure::NeedsSignIn {
                challenge: None,
            });
        };
        let access = Zeroizing::new(access);
        let expiring = creds.as_ref().is_some_and(|c| {
            c.refresh_token.is_some() && c.expires_at.is_some_and(|t| t <= unix_now() + EXPIRY_MARGIN_SECS)
        });
        if server.auth == "oauth" && expiring {
            return self.mcp_renew(server, &access).await.map(Some);
        }
        Ok(Some(access))
    }

    /// A new access token after `failed` was refused or expired. One renewal at a time: a caller that waited finds
    /// the token another one already renewed.
    async fn mcp_renew(&self, server: &StoredMcpServer, failed: &str) -> Result<Zeroizing<String>, Failure> {
        let needs = || Failure::NeedsSignIn {
            challenge: None,
        };
        if server.auth != "oauth" {
            return Err(needs());
        }
        let _one_at_a_time = self.mcp.refreshing.lock().await;
        let mut creds = self.mcp_credentials(&server.id)?.ok_or_else(needs)?;
        if let Some(current) = &creds.access_token {
            let fresh = creds.expires_at.is_none_or(|t| t > unix_now() + EXPIRY_MARGIN_SECS);
            if current != failed && fresh {
                return Ok(Zeroizing::new(current.clone()));
            }
        }
        let refresh_token = Zeroizing::new(creds.refresh_token.clone().ok_or_else(needs)?);
        let client = client_of(server).ok_or_else(needs)?;
        match oauth::refresh(&self.http, &client, creds.client_secret.as_deref(), &refresh_token).await {
            Ok(answer) => {
                creds.access_token = Some(answer.access_token.clone());
                if let Some(rotated) = &answer.refresh_token {
                    creds.refresh_token = Some(rotated.clone());
                }
                creds.expires_at = answer.expires_in.map(|s| unix_now().saturating_add(s));
                self.mcp_save_credentials(&server.id, &creds)?;
                Ok(Zeroizing::new(answer.access_token.clone()))
            }
            Err(RefreshError::Rejected) => Err(needs()),
            Err(RefreshError::Other(e)) => Err(e.into()),
        }
    }

    async fn mcp_attempt(
        &self,
        server: &StoredMcpServer,
        token: Option<&str>,
        op: &Op<'_>,
    ) -> Result<Output, McpError> {
        let endpoint = Endpoint {
            http: &self.http,
            url: &server.url,
            token,
            ids: &self.mcp.next_id,
        };
        let cached = self.mcp.sessions.lock().await.get(&server.id).cloned();
        let conn = if let Some(conn) = cached {
            conn
        } else {
            let conn = endpoint.initialize().await?;
            self.mcp.sessions.lock().await.insert(server.id.clone(), conn.clone());
            conn
        };
        match op {
            Op::ListTools => endpoint.list_tools(&conn).await.map(Output::Tools),
            Op::Call {
                tool,
                arguments,
            } => endpoint.call_tool(&conn, tool, arguments).await.map(Output::Result),
            Op::Proxy {
                tool,
                arguments,
                connection_id,
                session,
            } => {
                let (_, request) = endpoint.request("tools/call", &json!({"name": tool, "arguments": arguments}));
                let call = ProxyCall {
                    v: rewarden_proto::PROTOCOL_VERSION,
                    connection_id: (*connection_id).clone(),
                    endpoint: server.url.clone(),
                    headers: endpoint.headers(Some(&conn)),
                    request,
                };
                let answer = server_api::proxy_call(session, &call).await?;
                drop(call);
                if answer.session_id.is_some() && answer.session_id != conn.session_id {
                    let renewed = Connected {
                        session_id: answer.session_id.clone(),
                        protocol: conn.protocol.clone(),
                    };
                    self.mcp.sessions.lock().await.insert(server.id.clone(), renewed);
                }
                match answer.status {
                    401 => {
                        return Err(McpError::Unauthorized {
                            challenge: None,
                        });
                    }
                    404 if conn.session_id.is_some() => return Err(McpError::SessionGone),
                    200..=299 => {}
                    other => return Err(McpError::Status(other)),
                }
                let message = answer.response.ok_or_else(|| McpError::Protocol("no answer".to_owned()))?;
                let result = result_of(message)?;
                if result.is_object() {
                    Ok(Output::Result(result))
                } else {
                    Err(McpError::Protocol("the tool result is not an object".to_owned()))
                }
            }
        }
    }

    /// Runs one request on `server`: a session is opened when there is none, an expired token renewed once, an
    /// ended session reopened once. When no token works the server is marked as needing a sign-in.
    pub(crate) async fn mcp_run(&self, server: &StoredMcpServer, op: &Op<'_>) -> Result<Output, Failure> {
        let outcome = self.mcp_run_inner(server, op).await;
        match &outcome {
            Ok(_) if server.status != "ok" => {
                self.store.mcp_set_status(&server.id, "ok", None).ok();
            }
            Err(Failure::NeedsSignIn {
                ..
            }) => {
                self.store.mcp_set_status(&server.id, "needs_sign_in", Some("Sign in again.")).ok();
            }
            _ => {}
        }
        outcome
    }

    async fn mcp_run_inner(&self, server: &StoredMcpServer, op: &Op<'_>) -> Result<Output, Failure> {
        let mut token = self.mcp_token(server).await?;
        let (mut renewed, mut reopened) = (false, false);
        loop {
            match self.mcp_attempt(server, token.as_deref().map(String::as_str), op).await {
                Ok(output) => return Ok(output),
                Err(McpError::Unauthorized {
                    ..
                }) if !renewed && server.auth == "oauth" => {
                    renewed = true;
                    let failed = token.take().unwrap_or_default();
                    token = Some(self.mcp_renew(server, &failed).await?);
                }
                Err(McpError::Unauthorized {
                    challenge,
                }) => {
                    return Err(Failure::NeedsSignIn {
                        challenge,
                    });
                }
                Err(McpError::SessionGone) if !reopened => {
                    reopened = true;
                    self.mcp_forget_session(&server.id).await;
                }
                Err(e) => return Err(Failure::Mcp(e)),
            }
        }
    }

    // ---- what the report and the app see -----------------------------------------------------------------------

    /// The servers and tools to report to the Rewarden server.
    pub(crate) fn mcp_reports(&self) -> Vec<McpServerReport> {
        match self.store.mcp_servers() {
            Ok(servers) => super::reports(&servers),
            Err(e) => {
                log::warn!("could not read the MCP servers: {e}");
                Vec::new()
            }
        }
    }

    /// Tells the server about a change at once (when signed in); otherwise the next sync does.
    async fn mcp_report_now(&self) {
        if let Ok(session) = self.session() {
            self.report_services(&session).await;
        }
    }

    pub fn mcp_servers(&self) -> Result<Vec<McpServerView>, CoreError> {
        Ok(self.store.mcp_servers()?.iter().map(server_view).collect())
    }

    fn mcp_view(&self, id: &str) -> Result<McpServerView, CoreError> {
        self.store.mcp_server(id)?.as_ref().map(server_view).ok_or(CoreError::NotFound)
    }

    // ---- adding ------------------------------------------------------------------------------------------------

    /// A new server's record (not saved), checking the address and that it is not there yet.
    fn mcp_new(&self, url: &str, name: Option<&str>, auth: &str) -> Result<(Url, StoredMcpServer), CoreError> {
        let parsed = checked_url(url)?;
        let servers = self.store.mcp_servers()?;
        if servers.iter().any(|s| same_address(&s.url, &parsed)) {
            return Err(CoreError::invalid("That server is already added."));
        }
        if servers.len() >= MAX_SERVERS {
            return Err(CoreError::invalid(format!("At most {MAX_SERVERS} MCP servers can be added.")));
        }
        let name = display_name(name, &parsed);
        let taken: Vec<String> = servers.into_iter().map(|s| s.id).collect();
        let server = StoredMcpServer {
            id: new_server_id(&name, &taken),
            name,
            url: parsed.to_string(),
            auth: auth.to_owned(),
            client: None,
            tools: Vec::new(),
            heavy: Vec::new(),
            status: "ok".to_owned(),
            error: None,
            listed_at: None,
            added_at: unix_now(),
        };
        Ok((parsed, server))
    }

    /// Saves a server that just answered with its tools, and tells the Rewarden server.
    async fn mcp_save_connected(
        &self,
        mut server: StoredMcpServer,
        conn: Connected,
        tools: Vec<McpTool>,
    ) -> Result<McpServerView, CoreError> {
        let now = unix_now();
        server.tools = tools;
        server.listed_at = Some(now);
        "ok".clone_into(&mut server.status);
        server.error = None;
        self.store.mcp_put(&server)?;
        self.mcp.sessions.lock().await.insert(server.id.clone(), conn);
        self.mcp_report_now().await;
        Ok(server_view(&server))
    }

    /// Adds a server by its address. One that needs no sign-in is added at once; one that does gets a client
    /// registered and the address of its sign-in page.
    pub async fn mcp_add(&self, url: &str, name: Option<String>) -> Result<McpAddStep, CoreError> {
        // A sign-in that was abandoned is started again rather than refused as a duplicate.
        if let Ok(parsed) = checked_url(url)
            && let Some(waiting) = self
                .store
                .mcp_servers()?
                .into_iter()
                .find(|s| same_address(&s.url, &parsed) && s.status == "needs_sign_in" && s.auth == "oauth")
            && let Some(client) = client_of(&waiting)
        {
            return self.mcp_start_sign_in(&waiting.id, &client);
        }
        let (_, mut server) = self.mcp_new(url, name.as_deref(), "none")?;
        let endpoint = Endpoint {
            http: &self.http,
            url: &server.url,
            token: None,
            ids: &self.mcp.next_id,
        };
        let challenge = match endpoint.initialize().await {
            Ok(conn) => {
                let tools = endpoint.list_tools(&conn).await.map_err(|e| connect_error(&e))?;
                let view = self.mcp_save_connected(server, conn, tools).await?;
                return Ok(McpAddStep::Added {
                    server: view,
                });
            }
            Err(McpError::Unauthorized {
                challenge,
            }) => challenge,
            Err(e) => return Err(connect_error(&e)),
        };
        let client = self.mcp_register(&mut server, challenge.as_deref()).await?;
        self.mcp_start_sign_in(&server.id, &client)
    }

    /// Registers this phone with the authorization server of a server that asks for a sign-in, and saves the client
    /// (its secret, if it got one, with the tokens).
    async fn mcp_register(
        &self,
        server: &mut StoredMcpServer,
        challenge: Option<&str>,
    ) -> Result<OAuthClient, CoreError> {
        let parsed = checked_url(&server.url)?;
        let (client, secret) = oauth::register(&self.http, &parsed, challenge).await?;
        "oauth".clone_into(&mut server.auth);
        server.client = Some(serde_json::to_string(&client).map_err(|e| CoreError::storage(e.to_string()))?);
        "needs_sign_in".clone_into(&mut server.status);
        self.store.mcp_put(server)?;
        if secret.is_some() {
            self.mcp_save_credentials(
                &server.id,
                &Credentials {
                    access_token: None,
                    refresh_token: None,
                    expires_at: None,
                    client_secret: secret,
                },
            )?;
        }
        Ok(client)
    }

    fn mcp_start_sign_in(&self, id: &str, client: &OAuthClient) -> Result<McpAddStep, CoreError> {
        let (pending, authorize_url) = oauth::start(client)?;
        let raw = Zeroizing::new(serde_json::to_vec(&pending).map_err(|e| CoreError::storage(e.to_string()))?);
        self.store.secret_put(MCP_SIGN_IN_SERVICE, id, &raw)?;
        Ok(McpAddStep::NeedsSignIn {
            server_id: id.to_owned(),
            authorize_url,
        })
    }

    /// The browser came back to `dev.rewarden.android://mcp-oauth?code=…&state=…`: trades the code for tokens, then
    /// lists the tools.
    pub async fn mcp_finish_sign_in(&self, server_id: &str, redirect_url: &str) -> Result<McpServerView, CoreError> {
        let server = self.store.mcp_server(server_id)?.ok_or(CoreError::NotFound)?;
        let client = client_of(&server).ok_or_else(|| CoreError::invalid("This server is not signed in that way."))?;
        let pending: PendingSignIn = self
            .store
            .secret_get(MCP_SIGN_IN_SERVICE, server_id)?
            .map(Zeroizing::new)
            .and_then(|raw| serde_json::from_slice(&raw).ok())
            .ok_or_else(|| CoreError::invalid("No sign-in was started for this server. Start it again."))?;
        let redirect =
            Url::parse(redirect_url.trim()).map_err(|_| CoreError::invalid("That is not the sign-in's answer."))?;
        let expected = Url::parse(REDIRECT_URI).map_err(|_| CoreError::storage("bad redirect"))?;
        if redirect.scheme() != expected.scheme() || redirect.host_str() != expected.host_str() {
            return Err(CoreError::invalid("That is not the sign-in's answer."));
        }
        let query: HashMap<String, String> = redirect.query_pairs().into_owned().collect();
        if query.get("state") != Some(&pending.state) {
            return Err(CoreError::invalid("This answer is not for the sign-in that was started. Try again."));
        }
        if let Some(error) = query.get("error") {
            self.store.secret_delete(MCP_SIGN_IN_SERVICE, server_id)?;
            let why = query.get("error_description").unwrap_or(error);
            return Err(CoreError::service(format!(
                "The sign-in was not completed: {}",
                text::truncate_chars(&text::one_line(why), 200)
            )));
        }
        // RFC 9207: when the server names itself, it must be the one the sign-in started with.
        if query.get("iss").is_some_and(|iss| iss.trim_end_matches('/') != client.issuer) {
            return Err(CoreError::invalid("This answer comes from another sign-in server."));
        }
        let code =
            query.get("code").filter(|c| !c.is_empty()).ok_or_else(|| CoreError::invalid("The answer has no code."))?;
        let mut creds = self.mcp_credentials(server_id)?.unwrap_or_default();
        let answer = oauth::exchange(&self.http, &client, creds.client_secret.as_deref(), code, &pending).await?;
        creds.access_token = Some(answer.access_token.clone());
        creds.refresh_token.clone_from(&answer.refresh_token);
        creds.expires_at = answer.expires_in.map(|s| unix_now().saturating_add(s));
        self.mcp_save_credentials(server_id, &creds)?;
        self.store.secret_delete(MCP_SIGN_IN_SERVICE, server_id)?;
        self.mcp_forget_session(server_id).await;
        self.mcp_relist(&server).await
    }

    /// Lists the tools again and saves them (and reports them when they changed).
    async fn mcp_relist(&self, server: &StoredMcpServer) -> Result<McpServerView, CoreError> {
        match self.mcp_run(server, &Op::ListTools).await {
            Ok(Output::Tools(tools)) => {
                self.store.mcp_set_tools(&server.id, &tools, unix_now())?;
                self.mcp_report_now().await;
                self.mcp_view(&server.id)
            }
            Ok(Output::Result(_)) => Err(CoreError::storage("unexpected answer")),
            Err(f) => {
                if let Failure::Mcp(e) = &f {
                    self.store.mcp_set_status(&server.id, "error", Some(&e.describe())).ok();
                }
                Err(f.into_core(&server.name))
            }
        }
    }

    /// Adds a server with an access token the user pasted.
    pub async fn mcp_add_with_token(
        &self,
        url: &str,
        token: &str,
        name: Option<String>,
    ) -> Result<McpServerView, CoreError> {
        let token = token.trim();
        if token.is_empty() || token.len() > MAX_TOKEN_LEN || token.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(CoreError::invalid("That is not an access token."));
        }
        let (_, server) = self.mcp_new(url, name.as_deref(), "token")?;
        let endpoint = Endpoint {
            http: &self.http,
            url: &server.url,
            token: Some(token),
            ids: &self.mcp.next_id,
        };
        let connected = match endpoint.initialize().await {
            Ok(conn) => endpoint.list_tools(&conn).await.map(|tools| (conn, tools)),
            Err(e) => Err(e),
        };
        let (conn, tools) = match connected {
            Ok(found) => found,
            Err(McpError::Unauthorized {
                ..
            }) => return Err(CoreError::invalid("The server did not accept that token.")),
            Err(e) => return Err(connect_error(&e)),
        };
        self.mcp_save_credentials(
            &server.id,
            &Credentials {
                access_token: Some(token.to_owned()),
                refresh_token: None,
                expires_at: None,
                client_secret: None,
            },
        )?;
        self.mcp_save_connected(server, conn, tools).await
    }

    /// Connects again and lists the tools. A server whose sign-in ended gets a new sign-in started.
    pub async fn mcp_refresh(&self, id: &str) -> Result<McpAddStep, CoreError> {
        let mut server = self.store.mcp_server(id)?.ok_or(CoreError::NotFound)?;
        self.mcp_forget_session(id).await;
        match self.mcp_relist(&server).await {
            Ok(view) => Ok(McpAddStep::Added {
                server: view,
            }),
            Err(CoreError::ServiceNeedsAttention {
                ..
            }) if server.auth != "token" => {
                let client = match client_of(&server) {
                    Some(client) => client,
                    // A server that needed no sign-in now asks for one.
                    None => self.mcp_register(&mut server, None).await?,
                };
                self.mcp_start_sign_in(id, &client)
            }
            Err(
                CoreError::ServiceNeedsAttention {
                    ..
                }
                | CoreError::Service {
                    ..
                }
                | CoreError::Network {
                    ..
                },
            ) => Ok(McpAddStep::Added {
                server: self.mcp_view(id)?,
            }),
            Err(e) => Err(e),
        }
    }

    /// Removes a server: its tokens, its permissions and its tools in the report go; calls waiting for it are
    /// answered with an error.
    pub async fn mcp_remove(&self, id: &str) -> Result<(), CoreError> {
        if !self.store.mcp_remove(id, &grant_service(id))? {
            return Err(CoreError::NotFound);
        }
        self.mcp_forget_session(id).await;
        let session = self.session().ok();
        for row in self.store.pending_rows(unix_now())? {
            if row.kind != PendingKind::Request {
                continue;
            }
            let Ok(parked) = serde_json::from_slice::<ParkedRequest>(&row.payload) else {
                continue;
            };
            let about = matches!(&parked.request.call, rewarden_proto::gmail::ToolCall::Mcp(c) if c.server == id);
            if !about || !self.store.remove_pending(&row.id)? {
                continue;
            }
            self.notifier.item_resolved(row.id.clone());
            if let Some(session) = &session {
                let outcome = rewarden_proto::relay::RelayOutcome::Error {
                    message: "The user removed that MCP server from Rewarden.".to_owned(),
                };
                self.respond(session, &row.id, outcome).await.ok();
            }
        }
        self.mcp_report_now().await;
        Ok(())
    }

    /// Sends a tool's results through the Rewarden server (heavy) or not.
    pub fn mcp_set_heavy(&self, id: &str, tool: &str, heavy: bool) -> Result<(), CoreError> {
        if self.store.mcp_set_heavy(id, tool, heavy)? {
            Ok(())
        } else {
            Err(CoreError::NotFound)
        }
    }
}
