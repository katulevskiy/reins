package dev.reins.android.platform

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import dev.reins.android.MainActivity
import dev.reins.android.ui.signin.AccountRules

/**
 * Receives `com.reins2fa.app://sso-callback?…`, where the server's sign-in page ("Continue") sends the browser back to,
 * and hands it to [MainActivity]. Like [McpRedirectActivity], clearing the top of the task closes the Custom Tab the
 * page was shown in. Shows nothing itself.
 */
class SsoRedirectActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val data = intent?.data
        if (intent?.action == Intent.ACTION_VIEW && data != null && AccountRules.isSsoCallback(data.toString())) {
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
        /** The action [MainActivity] receives the callback with. */
        const val ACTION_SIGNED_IN = "dev.reins.android.SSO_SIGNED_IN"
    }
}
