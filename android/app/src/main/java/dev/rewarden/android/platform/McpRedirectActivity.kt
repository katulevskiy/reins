package dev.rewarden.android.platform

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import dev.rewarden.android.MainActivity
import dev.rewarden.android.ui.mcp.isMcpRedirect

/**
 * Receives `dev.rewarden.android://mcp-oauth?…`, where an MCP server's sign-in page sends the browser back to, and hands
 * it to [MainActivity]. Clearing the top of the task closes the Custom Tab the page was shown in, instead of stacking a
 * second app screen on top of it. Shows nothing itself.
 */
class McpRedirectActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val data = intent?.data
        if (intent?.action == Intent.ACTION_VIEW && data != null && isMcpRedirect(data.toString())) {
            startActivity(
                Intent(this, MainActivity::class.java)
                    .setAction(ACTION_SIGNED_IN)
                    .setData(data)
                    .addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP),
            )
        }
        finish()
    }

    companion object {
        /** The action [MainActivity] receives the redirect with. */
        const val ACTION_SIGNED_IN = "dev.rewarden.android.MCP_SIGNED_IN"
    }
}
