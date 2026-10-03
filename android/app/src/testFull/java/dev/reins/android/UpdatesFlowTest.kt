package dev.reins.android

import android.content.Context
import android.content.Intent
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.platform.update.FakeInstaller
import dev.reins.android.platform.update.FakeUpdateServer
import dev.reins.android.platform.update.UpdateNotifier
import dev.reins.android.platform.update.UpdateProvider
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** The in-app updater of the `full` build (the APK from reins2fa.com); the `play` build has none. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class UpdatesFlowTest : FlowHarness() {
    private val updates = FakeUpdateServer()
    private val installer = FakeInstaller()
    private val updatePrefs get() = context.getSharedPreferences("updates", Context.MODE_PRIVATE)

    @Before
    fun useTheseFakes() {
        UpdateProvider.fetcher = updates
        UpdateProvider.installer = installer
    }

    private fun awaitTextSub(text: String) = awaitText(text, substring = true)

    @Test
    fun theAppSchedulesOneBackgroundUpdateCheckOnlineEverySixHours() {
        launch()
        awaitTag("noActivity")
        scenario?.close()
        launch()
        awaitTag("noActivity")
        val work = androidx.work.WorkManager.getInstance(context).getWorkInfosForUniqueWork("app-update").get()
        assertEquals(1, work.size)
        val info = work.single()
        assertEquals(6 * 3600_000L, info.periodicityInfo?.repeatIntervalMillis)
        assertEquals(androidx.work.NetworkType.CONNECTED, info.constraints.requiredNetworkType)
    }

    @Test
    fun settingsShowTheVersionAndSayWhenTheAppIsUpToDate() {
        launch()
        tap("openSettings")
        awaitTag("appVersion")
        awaitTextSub("${BuildConfig.VERSION_NAME} (dev)")
        tap("checkUpdates")
        awaitText("Reins is up to date.")
        rule.onNodeWithTag("autoDownload").assertIsOn()
    }

    @Test
    fun aNewReleaseIsDownloadedVerifiedAndInstalledFromSettings() {
        launch()
        tap("openSettings")
        awaitTag("checkUpdates")
        val release = updates.publish(5, versionName = "0.2.0")
        tap("checkUpdates")
        awaitText("Update ready: 0.2.0 — Install")
        tap("installUpdate")
        awaitCore { installer.installed.size == 1 }
        assertEquals(release.file, installer.installed.single().name)
    }

    @Test
    fun theSettingsShowTheDownloadProgress() {
        val gate = java.util.concurrent.CountDownLatch(1)
        updates.gate = gate
        launch()
        tap("openSettings")
        awaitTag("checkUpdates")
        updates.publish(5, versionName = "0.2.0")
        tap("checkUpdates")
        try {
            awaitTag("updateProgress")
            awaitTextSub("Downloading Reins 0.2.0")
            awaitTextSub("50%")
        } finally {
            gate.countDown()
        }
        awaitText("Update ready: 0.2.0 — Install")
    }

    @Test
    fun turningAutomaticDownloadsOffIsRememberedAndOffersTheDownload() {
        launch()
        tap("openSettings")
        tap("autoDownload")
        rule.onNodeWithTag("autoDownload").assertIsOff()
        awaitCore { !updatePrefs.getBoolean("auto_download", true) }
        updates.publish(5, versionName = "0.2.0")
        tap("checkUpdates")
        awaitText("Reins 0.2.0 is available.")
        assertEquals(0, updates.apkCalls)
        tap("installUpdate")
        awaitCore { installer.installed.size == 1 }
    }

    @Test
    fun aFailedCheckSaysWhyAndCanBeRetried() {
        launch()
        tap("openSettings")
        awaitTag("checkUpdates")
        updates.failure = java.io.IOException("offline")
        tap("checkUpdates")
        awaitTextSub("Couldn't reach the update server")
        updates.failure = null
        tap("retryUpdate")
        awaitText("Reins is up to date.")
    }

    @Test
    fun aDownloadedUpdateIsOfferedOnTheMainScreenAndLaterHidesItForADay() {
        updates.publish(5, versionName = "0.2.0")
        launch()
        awaitTag("updatePrompt")
        awaitText("Reins 0.2.0 is ready to install.")
        tap("updatePromptLater")
        awaitGone("updatePrompt")
        awaitCore { updatePrefs.getLong("snoozed_version", 0) == 5L }
        scenario?.close()
        launch()
        awaitTag("noActivity")
        settle()
        assertFalse(has("updatePrompt"))
    }

    @Test
    fun thePromptInstallsTheUpdate() {
        updates.publish(5)
        launch()
        tap("updatePromptInstall")
        awaitCore { installer.installed.size == 1 }
    }

    @Test
    fun thePromptNeverCoversAnApprovalSheet() {
        updates.publish(5)
        openRequest(TestData.searchView())
        awaitCore { container.updates!!.state.value.showPrompt }
        settle()
        assertFalse(has("updatePrompt"))
        tap("closeSheet")
        awaitTag("updatePrompt")
    }

    @Test
    fun installingAsksToAllowInstallsFirstAndContinuesAfterwards() {
        installer.allowed = false
        updates.publish(5)
        launch()
        tap("updatePromptInstall")
        awaitTag("allowInstalls")
        rule.onNodeWithTag("allowInstalls").performClick()
        awaitCore { installer.permissionScreens == 1 }
        assertTrue(installer.installed.isEmpty())
        // Back from Android's setting with the permission given.
        installer.allowed = true
        scenario!!.moveToState(androidx.lifecycle.Lifecycle.State.STARTED)
        scenario!!.moveToState(androidx.lifecycle.Lifecycle.State.RESUMED)
        awaitCore { installer.installed.size == 1 }
    }

    @Test
    fun theUpdateNotificationOpensTheAppToThePromptEvenWhenSnoozed() {
        updates.publish(5)
        updatePrefs.edit().putLong("snoozed_version", 5).putLong("snoozed_until", Long.MAX_VALUE).commit()
        launch(Intent(context, MainActivity::class.java).setAction(UpdateNotifier.ACTION_OPEN_UPDATE))
        awaitTag("updatePrompt")
    }
}
