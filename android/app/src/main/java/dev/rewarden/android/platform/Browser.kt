package dev.rewarden.android.platform

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.browser.customtabs.CustomTabsIntent

/** Web pages the user has to visit (an MCP server's sign-in): a Custom Tab where the phone has one, else the browser. */
object Browser {
    /** False when nothing on the phone can show [url]. */
    fun open(context: Context, url: String): Boolean {
        val uri = Uri.parse(url)
        return try {
            CustomTabsIntent.Builder().setShowTitle(true).build().launchUrl(context, uri)
            true
        } catch (e: ActivityNotFoundException) {
            try {
                val view = Intent(Intent.ACTION_VIEW, uri)
                if (context !is Activity) view.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                context.startActivity(view)
                true
            } catch (e: ActivityNotFoundException) {
                false
            }
        }
    }
}
