package dev.reins.android.platform

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.browser.customtabs.CustomTabsClient
import androidx.browser.customtabs.CustomTabsIntent
import androidx.browser.customtabs.CustomTabsService

/** Web pages the user has to visit (an MCP server's sign-in): a Custom Tab where the phone has one, else the browser. */
object Browser {
    /**
     * False when nothing on the phone can show [url]. [ephemeral]: in a tab that neither reuses nor keeps the browser's
     * cookies, where the browser has such tabs (elsewhere an ordinary one).
     */
    fun open(context: Context, url: String, ephemeral: Boolean = false): Boolean {
        val uri = Uri.parse(url)
        return try {
            // A default browser without Custom Tabs otherwise opens a full browser even when another installed
            // browser supports them. Prefer the default when it supports tabs, then another available provider.
            val providers = context.packageManager.queryIntentServices(
                Intent(CustomTabsService.ACTION_CUSTOM_TABS_CONNECTION),
                0,
            ).map { it.serviceInfo.packageName }.distinct()
            val provider = CustomTabsClient.getPackageName(context, providers)
            val tab = CustomTabsIntent.Builder()
                .setShowTitle(true)
                .setShareState(CustomTabsIntent.SHARE_STATE_OFF)
                .apply { if (ephemeral && ephemeralSupported(context, provider)) setEphemeralBrowsingEnabled(true) }
                .build()
            provider?.let { tab.intent.setPackage(it) }
            if (context !is Activity) tab.intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            tab.launchUrl(context, uri)
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

    private fun ephemeralSupported(context: Context, provider: String?): Boolean =
        provider != null && try {
            CustomTabsClient.isEphemeralBrowsingSupported(context, provider)
        } catch (e: RuntimeException) {
            false
        }
}
