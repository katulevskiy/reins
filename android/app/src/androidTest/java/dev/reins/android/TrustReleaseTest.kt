package dev.reins.android

import android.app.Application
import android.content.Context
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.ext.junit.rules.ActivityScenarioRule
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.runner.AndroidJUnitRunner
import androidx.test.uiautomator.By
import androidx.test.uiautomator.BySelector
import androidx.test.uiautomator.Direction
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.UiObject2
import androidx.test.uiautomator.Until
import dev.reins.android.core.CoreFactory
import dev.reins.android.core.CoreProvider
import dev.reins.android.platform.ScreenLock
import dev.reins.core.AutopilotMode
import dev.reins.core.ModelState
import dev.reins.core.SessionInfo
import kotlinx.coroutines.CompletableDeferred
import org.junit.Assert.assertNotNull
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
        find(By.textStartsWith("Auto"))
        FakeCore.shared.profilesGate!!.complete(Unit)
        repeat(3) {
            click("Manual")
            find(By.textStartsWith("Manual").selected(true))
            click("Auto")
            find(By.textStartsWith("Auto").selected(true))
        }
        click("Bypass…")
        click("Cancel")
        click("Overnight…")
        click("Cancel")
        click("On-device model")
        click("Delete model")
        click("Cancel")
        find(By.desc("Close")).click()
        click("New")
        click("Cancel")
        click("Activity")
        click("Trust")
        find(By.textStartsWith("Auto").selected(true))
    }

    private fun click(text: String) = find(By.textStartsWith(text)).click()

    private fun find(selector: BySelector): UiObject2 {
        device.wait(Until.findObject(selector), 1_000)?.let { return it }
        val scroll = device.findObject(By.scrollable(true))
        scroll?.scroll(Direction.UP, 1f)
        repeat(6) {
            device.wait(Until.findObject(selector), 500)?.let { return it }
            scroll?.scroll(Direction.DOWN, 0.75f)
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
