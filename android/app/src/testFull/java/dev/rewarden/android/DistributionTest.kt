package dev.rewarden.android

import android.Manifest
import android.app.NotificationManager
import android.content.pm.PackageManager
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.rewarden.android.platform.PhoneBridge
import dev.rewarden.android.platform.update.UpdateNotifier
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** The `full` build (the APK from reins2fa.com): text messages, and it updates itself. See also UpdatesFlowTest. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class DistributionTest : FlowHarness() {
    @Test
    fun theApkAsksForTextMessagesAndInstallsItsOwnUpdates() {
        assertTrue(BuildConfig.HAS_SMS)
        assertTrue(BuildConfig.SELF_UPDATE)
        @Suppress("DEPRECATION")
        val info = context.packageManager.getPackageInfo(context.packageName, PackageManager.GET_PERMISSIONS or PackageManager.GET_RECEIVERS)
        val permissions = info.requestedPermissions.orEmpty().toSet()
        for (permission in listOf(Manifest.permission.READ_SMS, Manifest.permission.SEND_SMS, Manifest.permission.REQUEST_INSTALL_PACKAGES)) {
            assertTrue("$permission is declared", permission in permissions)
        }
        assertTrue(info.receivers.orEmpty().any { it.name.endsWith(".InstallResultReceiver") })
        assertNotNull(container.updates)
        val channel = context.getSystemService(NotificationManager::class.java).getNotificationChannel(UpdateNotifier.CHANNEL)
        assertNotNull(channel)
    }

    @Test
    fun theCoreIsOfferedTheTextMessages() {
        assertEquals(listOf("device_calendar", "device_contacts", "sms"), PhoneBridge(context).services())
    }

    @Test
    fun textMessagesAreListedAndConnectOnceAndroidAllowedThem() {
        core.serviceAdded.clear()
        launch()
        tap("integrations")
        tap("service:sms")
        awaitTag("allowDevice")
        kotlinx.coroutines.runBlocking { dev.rewarden.android.ui.services.ServiceViewModel(container, "sms").addDevice() }
        awaitCore { core.serviceAdded.isNotEmpty() }
        assertEquals(listOf("sms" to ""), core.serviceAdded.toList())
        awaitTag("account:this phone")
    }
}
