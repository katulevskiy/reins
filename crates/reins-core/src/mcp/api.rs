//! The MCP servers screens' operations, exported on `RewardenCore` (run on the core runtime like the rest).

use std::sync::Arc;

use zeroize::Zeroizing;

use super::{McpAddStep, McpServerView};
use crate::api::RewardenCore;
use crate::{CoreError, rt};

#[uniffi::export]
impl RewardenCore {
    /// The added MCP servers with their tools, oldest first.
    pub async fn mcp_servers(&self) -> Result<Vec<McpServerView>, CoreError> {
        let engine = Arc::clone(self.engine());
        rt::run(async move { engine.mcp_servers() }).await
    }

    /// Adds an MCP server by its address: added at once, or a sign-in page to open (its redirect goes to
    /// `mcp_finish_sign_in`).
    pub async fn mcp_add(&self, url: String, name: Option<String>) -> Result<McpAddStep, CoreError> {
        let engine = Arc::clone(self.engine());
        rt::run(async move { engine.mcp_add(&url, name).await }).await
    }

    /// The sign-in page redirected to `com.reins2fa.app://mcp-oauth?…`.
    pub async fn mcp_finish_sign_in(
        &self,
        server_id: String,
        redirect_url: String,
    ) -> Result<McpServerView, CoreError> {
        let engine = Arc::clone(self.engine());
        rt::run(async move { engine.mcp_finish_sign_in(&server_id, &redirect_url).await }).await
    }

    /// Adds an MCP server with an access token instead of a sign-in.
    pub async fn mcp_add_with_token(
        &self,
        url: String,
        token: String,
        name: Option<String>,
    ) -> Result<McpServerView, CoreError> {
        let engine = Arc::clone(self.engine());
        let token = Zeroizing::new(token);
        rt::run(async move { engine.mcp_add_with_token(&url, &token, name).await }).await
    }

    /// Connects again and lists the tools; a server whose sign-in ended gets a new sign-in page.
    pub async fn mcp_refresh(&self, id: String) -> Result<McpAddStep, CoreError> {
        let engine = Arc::clone(self.engine());
        rt::run(async move { engine.mcp_refresh(&id).await }).await
    }

    /// Removes a server with its tokens and permissions.
    pub async fn mcp_remove(&self, id: String) -> Result<(), CoreError> {
        let engine = Arc::clone(self.engine());
        rt::run(async move { engine.mcp_remove(&id).await }).await
    }

    /// Sends a tool's results through the Rewarden server (for large results) or not.
    pub async fn mcp_set_heavy(&self, id: String, tool: String, heavy: bool) -> Result<(), CoreError> {
        let engine = Arc::clone(self.engine());
        rt::run(async move { engine.mcp_set_heavy(&id, &tool, heavy) }).await
    }
}
