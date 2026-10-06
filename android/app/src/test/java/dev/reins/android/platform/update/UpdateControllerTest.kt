package dev.reins.android.platform.update

import android.app.NotificationManager
import android.content.Context
import android.content.pm.PackageInstaller
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.platform.AppNotifier
import dev.reins.android.platform.Foreground
import java.io.File
import java.io.IOException
import java.nio.file.Files
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class UpdateControllerTest {
    private val context: Context get() = ApplicationProvider.getApplicationContext()
    private val manager get() = context.getSystemService(NotificationManager::class.java)
    private val server = FakeUpdateServer()
    private val store = MemoryUpdateStore()
    private val installer = FakeInstaller()
    private val dir: File = Files.createTempDirectory("updates").toFile()
    private var now = 1_790_000_000_000L

    @Before
    fun setUp() {
        dev.reins.android.TestNativeKeys.install()
        shadowOf(context as android.app.Application).grantPermissions(android.Manifest.permission.POST_NOTIFICATIONS)
        AppNotifier(context) {}.createChannels()
        // The app creates it in the `full` build only; the updater's logic is the same in both.
        UpdateNotifier.createChannel(manager)
        Foreground.focused = false
    }

    @After
    fun tearDown() {
        Foreground.focused = false
    }

    private fun TestScope.controller(current: Long = 10): UpdateController {
        val dispatcher = UnconfinedTestDispatcher(testScheduler)
        return UpdateController(
            Updater({ server }, dir, store, current) { now },
            installer = { installer },
            notifier = UpdateNotifier(context),
            scope = CoroutineScope(backgroundScope.coroutineContext + dispatcher),
            io = dispatcher,
        ).also { it.start() }
    }

    private fun posted() = shadowOf(manager).allNotifications.filter { it.channelId == UpdateNotifier.CHANNEL }

    private fun text(n: android.app.Notification) = n.extras.getCharSequence("android.title").toString()

    // ---- background checks -------------------------------------------------------------------------------------------

    @Test
    fun `the background check downloads a new release and announces it once`() = runTest {
        val r = server.publish(11, versionName = "0.2.0")
        val c = controller()
        c.backgroundCheck()
        assertEquals(UpdateStatus.Ready(r), c.state.value.status)
        assertEquals(listOf("Reins 0.2.0 is ready to install"), posted().map(::text))
        c.backgroundCheck()
        controller().backgroundCheck()
        assertEquals(1, posted().size)
        assertEquals(1, server.apkCalls)
    }

    @Test
    fun `a later release is announced again`() = runTest {
        server.publish(11, versionName = "0.2.0")
        controller().backgroundCheck()
        manager.cancelAll()
        server.publish(12, versionName = "0.3.0")
        controller().backgroundCheck()
        assertEquals(listOf("Reins 0.3.0 is ready to install"), posted().map(::text))
    }

    @Test
    fun `without automatic downloads the release is announced but not downloaded`() = runTest {
        store.autoDownload = false
        val r = server.publish(11, versionName = "0.2.0")
        val c = controller()
        c.backgroundCheck()
        assertEquals(UpdateStatus.Available(r), c.state.value.status)
        assertEquals(0, server.apkCalls)
        assertEquals(listOf("Reins 0.2.0 is available"), posted().map(::text))
        assertEquals(emptyList<String>(), dir.list().orEmpty().toList())
    }

    @Test
    fun `nothing is announced when the app is up to date or the server fails`() = runTest {
        server.publish(10)
        controller().backgroundCheck()
        server.failure = IOException("offline")
        controller().backgroundCheck()
        assertEquals(0, posted().size)
    }

    @Test
    fun `a snoozed release is not announced`() = runTest {
        val r = server.publish(11)
        val c = controller()
        c.backgroundCheck()
        manager.cancelAll()
        store.notifiedVersion = 0
        c.later()
        c.backgroundCheck()
        assertEquals(0, posted().size)
        assertTrue(c.state.value.snoozed)
        assertEquals(r.versionCode, store.snoozedVersion)
    }

    @Test
    fun `a corrupt download is not announced`() = runTest {
        val r = server.publish(11)
        server.files[r.file] = FakeUpdateServer.apkBytes(3, r.size.toInt())
        val c = controller()
        c.backgroundCheck()
        assertEquals(0, posted().size)
        assertEquals(emptyList<String>(), dir.list().orEmpty().toList())
    }

    // ---- what the user sees -------------------------------------------------------------------------------------------

    @Test
    fun `a check by hand says up to date, available, ready or what went wrong`() = runTest {
        server.publish(10)
        val c = controller()
        c.checkNow()
        assertEquals(UpdateStatus.UpToDate, c.state.value.status)

        server.failure = IOException("offline")
        c.checkNow()
        val failed = c.state.value.status as UpdateStatus.Failed
        assertTrue(failed.message.startsWith("Couldn't reach the update server"))

        server.failure = null
        val r = server.publish(11)
        c.retry()
        assertEquals(UpdateStatus.Ready(r), c.state.value.status)
    }

    @Test
    fun `the prompt shows a ready release until it is snoozed, and a notification tap brings it back`() = runTest {
        server.publish(11)
        val c = controller()
        c.checkNow()
        assertTrue(c.state.value.showPrompt)
        c.later()
        assertFalse(c.state.value.showPrompt)
        // Still snoozed after a restart.
        val again = controller()
        again.checkNow()
        assertFalse(again.state.value.showPrompt)
        again.openFromNotification()
        assertTrue(again.state.value.showPrompt)
    }

    @Test
    fun `a download from before is offered on start without a check`() = runTest {
        val r = server.publish(11)
        controller().checkNow()
        val c = controller()
        assertEquals(UpdateStatus.Ready(r), c.state.value.status)
        assertEquals(1, server.manifestCalls)
    }

    @Test
    fun `foreground checks are throttled to every three hours`() = runTest {
        server.publish(10)
        val c = controller()
        c.onForeground()
        c.onForeground()
        assertEquals(1, server.manifestCalls)
        now += 3 * 3600_000L
        c.onForeground()
        assertEquals(2, server.manifestCalls)
    }

    @Test
    fun `a failed foreground check stays quiet`() = runTest {
        server.failure = IOException("offline")
        val c = controller()
        c.onForeground()
        assertEquals(UpdateStatus.Idle, c.state.value.status)
    }

    @Test
    fun `turning automatic downloads off is remembered, turning them on downloads what is available`() = runTest {
        store.autoDownload = false
        val r = server.publish(11)
        val c = controller()
        assertFalse(c.state.value.autoDownload)
        c.checkNow()
        assertEquals(UpdateStatus.Available(r), c.state.value.status)
        c.setAutoDownload(true)
        assertTrue(store.autoDownload)
        assertEquals(UpdateStatus.Ready(r), c.state.value.status)
        c.setAutoDownload(false)
        assertFalse(store.autoDownload)
    }

    // ---- installing ---------------------------------------------------------------------------------------------------

    @Test
    fun `install hands the verified file to the package installer`() = runTest {
        val r = server.publish(11)
        val c = controller()
        c.checkNow()
        c.install()
        assertEquals(listOf(File(dir, r.file)), installer.installed)
        assertEquals(UpdateStatus.Installing(r), c.state.value.status)
        // Android's confirmation screen pauses and resumes the app; the install stays in progress.
        now += 4 * 3600_000L
        c.onForeground()
        assertEquals(UpdateStatus.Installing(r), c.state.value.status)
    }

    @Test
    fun `install of an available release downloads it first`() = runTest {
        store.autoDownload = false
        val r = server.publish(11)
        val c = controller()
        c.checkNow()
        c.install()
        assertEquals(listOf(File(dir, r.file)), installer.installed)
    }

    @Test
    fun `a file that changed since the download is not installed`() = runTest {
        val r = server.publish(11)
        val c = controller()
        c.checkNow()
        File(dir, r.file).writeBytes(FakeUpdateServer.apkBytes(77, r.size.toInt()))
        c.install()
        assertEquals(emptyList<File>(), installer.installed)
        assertTrue(c.state.value.status is UpdateStatus.Failed)
        assertFalse(File(dir, r.file).exists())
    }

    @Test
    fun `without the install permission the user is asked, sent to settings and the install continues on return`() = runTest {
        installer.allowed = false
        server.publish(11)
        val c = controller()
        c.checkNow()
        c.install()
        assertTrue(c.state.value.askPermission)
        assertEquals(emptyList<File>(), installer.installed)
        c.openPermissionSettings(context)
        assertFalse(c.state.value.askPermission)
        assertEquals(1, installer.permissionScreens)
        installer.allowed = true
        c.onForeground()
        assertEquals(1, installer.installed.size)
    }

    @Test
    fun `coming back without the permission explains why nothing happened`() = runTest {
        installer.allowed = false
        val r = server.publish(11)
        val c = controller()
        c.checkNow()
        c.install()
        c.openPermissionSettings(context)
        c.onForeground()
        val failed = c.state.value.status as UpdateStatus.Failed
        assertEquals(r, failed.release)
        assertTrue(failed.message.contains("allow"))
        assertEquals(emptyList<File>(), installer.installed)
    }

    @Test
    fun `install results are turned into clear outcomes`() = runTest {
        val r = server.publish(11)
        val c = controller()
        c.checkNow()
        c.install()
        c.onInstallResult(PackageInstaller.STATUS_FAILURE_ABORTED, null)
        assertEquals(UpdateStatus.Ready(r), c.state.value.status)

        c.install()
        c.onInstallResult(PackageInstaller.STATUS_FAILURE_CONFLICT, "INSTALL_FAILED_UPDATE_INCOMPATIBLE: signatures do not match")
        val failed = c.state.value.status as UpdateStatus.Failed
        assertTrue(failed.message, failed.message.contains("signed with a different key"))
        assertTrue(c.state.value.showPrompt)

        c.install()
        c.onInstallResult(PackageInstaller.STATUS_FAILURE_INVALID, "bad apk")
        assertTrue((c.state.value.status as UpdateStatus.Failed).message.contains("not a valid app"))
        assertFalse(File(dir, r.file).exists())
    }

    @Test
    fun `install failure messages`() {
        assertNull(installFailureMessage(PackageInstaller.STATUS_FAILURE_ABORTED, null))
        assertTrue(installFailureMessage(PackageInstaller.STATUS_FAILURE_STORAGE, null)!!.contains("space"))
        assertTrue(installFailureMessage(PackageInstaller.STATUS_FAILURE_INCOMPATIBLE, null)!!.contains("this phone"))
        assertTrue(installFailureMessage(PackageInstaller.STATUS_FAILURE_BLOCKED, null)!!.contains("blocked"))
        assertTrue(installFailureMessage(PackageInstaller.STATUS_FAILURE, "boom")!!.contains("boom"))
    }
}
