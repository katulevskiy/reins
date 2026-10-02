package dev.rewarden.android

import android.Manifest
import android.app.NotificationManager
import android.content.Intent
import android.content.pm.PackageManager
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.rewarden.android.platform.PhoneBridge
import dev.rewarden.android.platform.update.UpdateNotifier
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The `play` build (Google Play): no SMS permissions or text-message integration (Play allows them only to default SMS
 * apps), and no updater or REQUEST_INSTALL_PACKAGES (Play installs the updates).
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class DistributionTest : FlowHarness() {
    @Test
    fun theManifestHasNoneOfThePermissionsPlayRestricts() {
        assertFalse(BuildConfig.HAS_SMS)
        assertFalse(BuildConfig.SELF_UPDATE)
        @Suppress("DEPRECATION")
        val info = context.packageManager.getPackageInfo(context.packageName, PackageManager.GET_PERMISSIONS or PackageManager.GET_RECEIVERS)
        val permissions = info.requestedPermissions.orEmpty().toSet()
        val restricted = listOf(
            Manifest.permission.READ_SMS,
            Manifest.permission.SEND_SMS,
            Manifest.permission.RECEIVE_SMS,
            Manifest.permission.REQUEST_INSTALL_PACKAGES,
            Manifest.permission.QUERY_ALL_PACKAGES,
            Manifest.permission.SCHEDULE_EXACT_ALARM,
            Manifest.permission.USE_EXACT_ALARM,
            Manifest.permission.ACCESS_BACKGROUND_LOCATION,
            Manifest.permission.READ_CALL_LOG,
            Manifest.permission.MANAGE_EXTERNAL_STORAGE,
        )
        assertEquals(emptyList<String>(), restricted.filter { it in permissions })
        assertFalse(info.receivers.orEmpty().any { it.name.endsWith(".InstallResultReceiver") })
    }

    @Test
    fun theCoreIsNotOfferedTextMessagesEvenWithThePermissions() {
        assertEquals(listOf("device_calendar", "device_contacts"), PhoneBridge(context).services())
        shadowOf(context as android.app.Application).grantPermissions(Manifest.permission.READ_SMS, Manifest.permission.SEND_SMS)
        assertFalse(PhoneBridge(context).permitted(PhoneBridge.SMS))
    }

    @Test
    fun textMessagesAreNotListedEvenThoughTheCoreCataloguesThem() {
        assertTrue(core.catalogue.any { it.service == "sms" })
        launch()
        tap("integrations")
        awaitTag("service:device_contacts")
        assertFalse(has("service:sms"))
        assertTrue(container.state.services.value.none { it.service == "sms" })
    }

    @Test
    fun settingsShowTheVersionButNoUpdater() {
        assertNull(container.updates)
        launch()
        tap("openSettings")
        awaitTag("appVersion")
        awaitText("Google Play keeps Rewarden up to date.")
        assertFalse(has("checkUpdates"))
        assertFalse(has("autoDownload"))
        assertFalse(showsText("UPDATES"))
        awaitText("VERSION")
    }

    @Test
    fun thereIsNoBackgroundUpdateCheckNorUpdatesChannel() {
        launch()
        awaitTag("noActivity")
        val work = androidx.work.WorkManager.getInstance(context).getWorkInfosForUniqueWork("app-update").get()
        assertTrue(work.isEmpty())
        assertNull(context.getSystemService(NotificationManager::class.java).getNotificationChannel(UpdateNotifier.CHANNEL))
    }

    @Test
    fun anUpdateLinkOpensTheAppAsUsual() {
        launch(Intent(context, MainActivity::class.java).setAction(UpdateNotifier.ACTION_OPEN_UPDATE))
        awaitTag("noActivity")
        assertFalse(has("updatePrompt"))
    }
}
