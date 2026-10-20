package dev.reins.android

import android.app.Application
import android.content.Context
import android.os.SystemClock
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.ext.junit.rules.ActivityScenarioRule
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.runner.AndroidJUnitRunner
import androidx.test.uiautomator.By
import androidx.test.uiautomator.BySelector
import androidx.test.uiautomator.Direction
import androidx.test.uiautomator.StaleObjectException
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.UiObject2
import androidx.test.uiautomator.Until
import dev.reins.android.core.CoreFactory
import dev.reins.android.core.CoreProvider
import dev.reins.android.platform.ScreenLock
import dev.reins.android.platform.update.UpdateFetcher
import dev.reins.android.platform.update.UpdateProvider
import dev.reins.core.AccountView
import dev.reins.core.AutopilotMode
import dev.reins.core.ModelState
import dev.reins.core.SessionInfo
import kotlinx.coroutines.CompletableDeferred
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Install the fake before the real Application starts, so this exercises the complete production navigation. */
class TrustTestRunner : AndroidJUnitRunner() {
    override fun onStart() {
        uiAutomation.grantRuntimePermission(targetContext.packageName, android.Manifest.permission.POST_NOTIFICATIONS)
        super.onStart()
    }

    override fun newApplication(cl: ClassLoader, className: String, context: Context): Application {
        val core = FakeCore.shared
        core.session = SessionInfo("http://127.0.0.1:8000", "test@example.com")
        core.apGlobal = FakeCore.ApRow(mode = AutopilotMode.AUTO)
        core.model = TestData.modelStatus(ModelState.INSTALLED)
        core.profilesGate = CompletableDeferred()
        CoreProvider.factory = CoreFactory { core }
        ScreenLock.check = { true }
        UpdateProvider.fetcher = object : UpdateFetcher {
            override fun manifest() = """{"versionCode":1,"versionName":"test","build":"test","file":"test.apk","sha256":"0000000000000000000000000000000000000000000000000000000000000000","size":1,"published_at":1}"""
            override fun apk(file: String): java.io.InputStream = throw AssertionError("Unexpected APK download")
        }
        return super.newApplication(cl, className, context)
    }
}

/** Uses accessibility to drive the actual optimized UI, without linking Compose's test runtime into the app. */
@RunWith(AndroidJUnit4::class)
class TrustReleaseTest {
    @get:Rule val activity = ActivityScenarioRule(MainActivity::class.java)
    private val device get() = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())

    @Test
    fun trustSurvivesAsyncRefreshesAndItsSheetsStillWork() {
        click("Trust")
        find(By.text("Auto"))
        FakeCore.shared.profilesGate!!.complete(Unit)
        repeat(3) {
            click("Manual")
            awaitMode(AutopilotMode.MANUAL)
            click("Auto")
            awaitMode(AutopilotMode.AUTO)
        }
        click("Bypass…")
        click("Cancel")
        click("Overnight…")
        click("Cancel")
        click("On-device model")
        click("Delete model")
        click("Cancel")
        click(By.desc("Close"))
        click("New")
        click("Cancel")
        click("Activity")
        click("Trust")
        find(By.text("Auto"))
        awaitMode(AutopilotMode.AUTO)

        // Exercise the new typed permission DTOs through the same optimized production navigation.
        // Preview is read-only: no provider account, policy save or owner-authentication step is involved.
        FakeCore.shared.accounts = listOf(AccountView("gmail", "synthetic@example.com", 1_700_000_100))
        click("Account and temporary permissions")
        click("View Gmail permissions")
        find(By.text("Account permissions"))
        find(By.text("synthetic@example.com"))
        click("Allow searching and reading")
        click("Review changes")
        find(By.text("Review permissions"))
        find(By.text("Claude Code"))
        find(By.text("Search mail: Allow automatically"))
        assertTrue(find(By.text("All in this account")).visibleBounds.height() > 0)
        assertTrue(find(By.text("At most 20 further uses for each automatic operation.")).visibleBounds.height() > 0)
        assertTrue(FakeCore.shared.savedPermissionDrafts.isEmpty())
        assertTrue(FakeCore.shared.delegationRecords.isEmpty())
        device.pressBack() // Discard the preview and keep the editor's unsaved draft.
        find(By.text("Account permissions"))
        device.pressBack()
        find(By.text("Permissions"))
        device.pressBack()
        find(By.text("Auto"))
    }

    private fun awaitMode(mode: AutopilotMode) {
        val deadline = SystemClock.uptimeMillis() + 5_000
        while (FakeCore.shared.apGlobal.mode != mode && SystemClock.uptimeMillis() < deadline) SystemClock.sleep(50)
        assertEquals(mode, FakeCore.shared.apGlobal.mode)
    }

    private fun click(text: String) = click(By.text(text))

    private fun click(selector: BySelector) {
        repeat(3) {
            device.waitForIdle(1_000)
            try {
                find(selector).click()
                return
            } catch (_: StaleObjectException) {
                // Async state updates can replace an accessibility node between lookup and the tap.
            }
        }
        find(selector).click()
    }

    private fun find(selector: BySelector): UiObject2 {
        device.wait(Until.findObject(selector), 1_000)?.let { return it }
        device.findObject(By.scrollable(true))?.scroll(Direction.UP, 1f)
        repeat(6) {
            device.wait(Until.findObject(selector), 500)?.let { return it }
            device.findObject(By.scrollable(true))?.scroll(Direction.DOWN, 0.75f)
        }
        val node = device.wait(Until.findObject(selector), 5_000)
        if (node == null) {
            val hierarchy = java.io.ByteArrayOutputStream()
            device.dumpWindowHierarchy(hierarchy)
            assertNotNull("No visible control matching $selector\n$hierarchy", node)
        }
        return node!!
    }
}
