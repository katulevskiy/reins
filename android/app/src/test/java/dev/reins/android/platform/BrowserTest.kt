package dev.reins.android.platform

import android.app.Application
import android.content.Intent
import android.content.pm.ActivityInfo
import android.content.pm.ResolveInfo
import android.content.pm.ServiceInfo
import android.net.Uri
import androidx.browser.customtabs.CustomTabsIntent
import androidx.browser.customtabs.CustomTabsService
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class BrowserTest {
    private val context: Application = ApplicationProvider.getApplicationContext()
    private val packages = shadowOf(context.packageManager)

    private fun defaultBrowser(name: String) {
        val info = ResolveInfo().apply {
            activityInfo = ActivityInfo().apply { packageName = name; this.name = "$name.BrowserActivity" }
        }
        packages.addResolveInfoForIntent(Intent(Intent.ACTION_VIEW, Uri.parse("http://")), info)
    }

    private fun tabProvider(name: String) {
        val info = ResolveInfo().apply {
            serviceInfo = ServiceInfo().apply { packageName = name; this.name = "$name.CustomTabsService" }
        }
        val intent = Intent(CustomTabsService.ACTION_CUSTOM_TABS_CONNECTION)
        packages.addResolveInfoForIntent(intent, info)
        packages.addResolveInfoForIntent(Intent(intent).setPackage(name), info)
    }

    @Test
    fun anUnsupportedDefaultBrowserDoesNotHideAnotherBrowsersCustomTab() {
        defaultBrowser("plain.browser")
        tabProvider("tabs.browser")
        assertTrue(Browser.open(context, "https://app.reins2fa.com/identity/connect/authorize"))
        val opened = shadowOf(context).nextStartedActivity
        assertEquals("tabs.browser", opened.`package`)
        assertEquals(CustomTabsIntent.SHARE_STATE_OFF, opened.getIntExtra(CustomTabsIntent.EXTRA_SHARE_STATE, -1))
        assertTrue(opened.flags and Intent.FLAG_ACTIVITY_NEW_TASK != 0)
    }

    @Test
    fun theDefaultBrowserIsPreferredWhenItSupportsTabs() {
        defaultBrowser("default.browser")
        tabProvider("other.browser")
        tabProvider("default.browser")
        assertTrue(Browser.open(context, "https://app.reins2fa.com/identity/connect/authorize"))
        assertEquals("default.browser", shadowOf(context).nextStartedActivity.`package`)
    }
}
