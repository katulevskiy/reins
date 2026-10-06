package dev.reins.android.platform

import android.content.Context
import dev.reins.android.state.AppState
import dev.reins.android.ui.common.userMessage
import dev.reins.android.ui.mcp.isMcpRedirect
import dev.reins.core.McpServerView
import dev.reins.core.ReinsCoreInterface
import kotlin.coroutines.cancellation.CancellationException

/** How a sign-in to an MCP server ended. */
sealed interface McpSignInResult {
    data class Done(val server: McpServerView) : McpSignInResult

    data class Failed(val serverId: String, val message: String) : McpSignInResult

    /** Not the redirect, or no sign-in was waiting for one: nothing was sent to the core. */
    data object Ignored : McpSignInResult
}

/**
 * The sign-in to an MCP server that waits for its browser page to come back. The server's id is kept on disk (it is not
 * secret) so that a sign-in still finishes when Android stopped the app while the page was open. Each redirect is used
 * once; the core checks that it belongs to the sign-in it started (state, PKCE).
 */
class McpSignIn(
    context: Context,
    private val core: () -> ReinsCoreInterface,
    private val state: AppState,
    private val now: () -> Long = System::currentTimeMillis,
) {
    private val prefs = SealedPreferences(context, "mcp_sign_in")

    /** The sign-in page of [serverId] is about to open. */
    fun begin(serverId: String) {
        prefs.edit().putString(KEY_SERVER, serverId).putLong(KEY_STARTED, now()).commit()
    }

    /** The server whose sign-in waits for its redirect, unless it was started too long ago. */
    fun pending(): String? {
        val id = prefs.getString(KEY_SERVER, null) ?: return null
        return id.takeIf { now() - prefs.getLong(KEY_STARTED, 0) in 0..MAX_AGE_MILLIS }
    }

    fun clear() {
        prefs.edit().clear().commit()
    }

    /** Hands [redirect] to the core for the waiting sign-in, then reloads the servers. */
    suspend fun finish(redirect: String): McpSignInResult {
        if (!isMcpRedirect(redirect)) return McpSignInResult.Ignored
        val epoch = state.accountEpoch.value
        val id = pending() ?: return McpSignInResult.Ignored
        clear()
        val result = try {
            McpSignInResult.Done(core().mcpFinishSignIn(id, redirect))
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            McpSignInResult.Failed(id, e.userMessage())
        }
        try {
            val servers = core().mcpServers()
            if (state.isCurrent(epoch)) state.setMcpServers(servers)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            // The list is reloaded on the next refresh.
        }
        return if (state.isCurrent(epoch)) result else McpSignInResult.Ignored
    }

    private companion object {
        const val KEY_SERVER = "server_id"
        const val KEY_STARTED = "started_at"

        /** A sign-in left open longer than this is not finished by a late redirect. */
        const val MAX_AGE_MILLIS = 30L * 60 * 1000
    }
}
