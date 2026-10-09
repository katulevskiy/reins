package dev.reins.android.platform

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.activity.result.ActivityResultLauncher
import androidx.browser.auth.AuthTabIntent
import androidx.browser.customtabs.CustomTabsClient
import androidx.browser.customtabs.CustomTabsIntent
import androidx.browser.customtabs.CustomTabsService

/** Web pages the user has to visit (sign-in pages): a Custom Tab or an Auth Tab where the phone has one, else the browser. */
object Browser {
    /**
     * False when nothing on the phone can show [url]. [ephemeral]: in a tab that neither reuses nor keeps the browser's
     * cookies, where the browser has such tabs (elsewhere an ordinary one).
     */
    fun open(context: Context, url: String, ephemeral: Boolean = false): Boolean {
        val uri = Uri.parse(url)
        return try {
            val provider = provider(context)
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

    /**
     * A sign-in page that ends at [scheme]: in an Auth Tab where the browser has one (Chrome 137 and later), which
     * opens over the app and hands the address it ends at to [launcher]'s callback, with no trip through another task.
     * Elsewhere a Custom Tab like [open], whose ending address comes back through the app's redirect activity.
     * [ephemeral] as for [open]: the page cannot sign in silently with the browser's last session.
     */
    fun openSignIn(
        context: Context,
        url: String,
        scheme: String,
        launcher: ActivityResultLauncher<Intent>,
        ephemeral: Boolean = false,
    ): Boolean {
        val provider = try { provider(context) } catch (e: RuntimeException) { null }
        if (provider == null || !CustomTabsClient.isAuthTabSupported(context, provider)) return open(context, url, ephemeral)
        return try {
            val tab = AuthTabIntent.Builder()
                .apply { if (ephemeral && ephemeralSupported(context, provider)) setEphemeralBrowsingEnabled(true) }
                .build()
            tab.intent.setPackage(provider)
            tab.launch(launcher, Uri.parse(url), scheme)
            true
        } catch (e: ActivityNotFoundException) {
            open(context, url, ephemeral)
        }
    }

    private fun ephemeralSupported(context: Context, provider: String?): Boolean =
        provider != null && try {
            CustomTabsClient.isEphemeralBrowsingSupported(context, provider)
        } catch (e: RuntimeException) {
            false
        }

    // A default browser without Custom Tabs otherwise opens a full browser even when another installed browser
    // supports them. Prefer the default when it supports tabs, then another available provider.
    private fun provider(context: Context): String? {
        val providers = context.packageManager.queryIntentServices(
            Intent(CustomTabsService.ACTION_CUSTOM_TABS_CONNECTION),
            0,
        ).map { it.serviceInfo.packageName }.distinct()
        return CustomTabsClient.getPackageName(context, providers)
    }
}
