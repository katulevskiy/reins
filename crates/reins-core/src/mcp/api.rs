//! The MCP servers screens' operations, exported on `ReinsCore` (run on the core runtime like the rest).

use std::sync::Arc;

use zeroize::Zeroizing;

use super::{McpAddStep, McpServerView};
use crate::api::ReinsCore;
use crate::{CoreError, rt};

#[uniffi::export]
impl ReinsCore {
    /// The added MCP servers with their tools, oldest first.
    pub async fn mcp_servers(&self) -> Result<Vec<McpServerView>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.mcp_servers() }).await;
            runtime.finish(&engine, result).await
        })
        .await
    }

    /// Adds an MCP server by its address: added at once, or a sign-in page to open (its redirect goes to
    /// `mcp_finish_sign_in`).
    pub async fn mcp_add(&self, url: String, name: Option<String>) -> Result<McpAddStep, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.mcp_add(&url, name).await }).await;
            runtime.finish(&engine, result).await
        })
        .await
    }

    /// The sign-in page redirected to `com.reins2fa.app://mcp-oauth?…`.
    pub async fn mcp_finish_sign_in(
        &self,
        server_id: String,
        redirect_url: String,
    ) -> Result<McpServerView, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.mcp_finish_sign_in(&server_id, &redirect_url).await }).await;
            runtime.finish(&engine, result).await
        })
        .await
    }

    /// Adds an MCP server with an access token instead of a sign-in.
    pub async fn mcp_add_with_token(
        &self,
        url: String,
        token: String,
        name: Option<String>,
    ) -> Result<McpServerView, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let token = Zeroizing::new(token);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.mcp_add_with_token(&url, &token, name).await }).await;
            runtime.finish(&engine, result).await
        })
        .await
    }

    /// Connects again and lists the tools; a server whose sign-in ended gets a new sign-in page.
    pub async fn mcp_refresh(&self, id: String) -> Result<McpAddStep, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.mcp_refresh(&id).await }).await;
            runtime.finish(&engine, result).await
        })
        .await
    }

    /// Removes a server with its tokens and permissions.
    pub async fn mcp_remove(&self, id: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.mcp_remove(&id).await }).await;
            runtime.finish(&engine, result).await
        })
        .await
    }

    /// Sends a tool's results through the Reins server (for large results) or not.
    pub async fn mcp_set_heavy(&self, id: String, tool: String, heavy: bool) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.mcp_set_heavy(&id, &tool, heavy) }).await;
            runtime.finish(&engine, result).await
        })
        .await
    }
}
