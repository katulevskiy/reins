package dev.reins.android.platform

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import dev.reins.android.MainActivity
import dev.reins.android.ui.services.isXRedirect

/**
 * Receives `com.reins2fa.app://x-oauth?…`, where X's sign-in page sends the browser back to, and hands it to
 * [MainActivity]. As with [McpRedirectActivity], clearing the top of the task closes the Custom Tab the page was shown
 * in. Shows nothing itself.
 */
class XRedirectActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val data = intent?.data
        if (intent?.action == Intent.ACTION_VIEW && data != null && isXRedirect(data.toString())) {
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
        const val ACTION_SIGNED_IN = "dev.reins.android.X_SIGNED_IN"
    }
}
