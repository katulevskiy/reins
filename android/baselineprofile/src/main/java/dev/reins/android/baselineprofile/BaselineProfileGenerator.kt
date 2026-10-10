package dev.reins.android.baselineprofile

import androidx.benchmark.macro.MacrobenchmarkScope
import androidx.benchmark.macro.junit4.BaselineProfileRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.uiautomator.By
import androidx.test.uiautomator.Until
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

private const val PACKAGE = "com.reins2fa.app"
private const val WAIT_MS = 15_000L

/**
 * What the app runs when it opens and when a request is answered, compiled ahead of time on the phone. Run on a
 * device or emulator where the app is signed in; a request waiting there (`reins test` on a paired computer) adds
 * the approval sheet, otherwise the connect page stands in for it. See android/README.md.
 */
@RunWith(AndroidJUnit4::class)
class BaselineProfileGenerator {
    @get:Rule
    val rule = BaselineProfileRule()

    /** Opening the app up to the first screen; these classes also go first in the APK's dex files. */
    @Test
    fun startup() = rule.collect(packageName = PACKAGE, includeInStartupProfile = true) {
        pressHome()
        startActivityAndWait()
        mainScreen()
    }

    /** Opening a request (or the connect page), the tabs and Settings. */
    @Test
    fun openRequestAndTabs() = rule.collect(packageName = PACKAGE) {
        pressHome()
        startActivityAndWait()
        mainScreen()
        if (!device.hasObject(By.desc("Close"))) {
            val review = device.findObject(By.text("Review")) ?: device.findObject(By.text("Connect a computer"))
            review?.click()
        }
        device.wait(Until.hasObject(By.desc("Close")), WAIT_MS)
        device.waitForIdle()
        device.pressBack()
        mainScreen()
        for (tab in listOf("Grants", "Integrations", "Activity")) {
            device.findObject(By.text(tab))?.click()
            device.waitForIdle()
        }
        device.findObject(By.desc("Settings"))?.click()
        device.waitForIdle()
        device.pressBack()
    }

    /** The tabs, or the sheet of the one request waiting (it opens by itself and covers them). */
    private fun MacrobenchmarkScope.mainScreen() {
        val shown = device.wait(Until.hasObject(By.text("Grants")), WAIT_MS) || device.hasObject(By.desc("Close"))
        check(shown) { "the app is not signed in on this device" }
        device.waitForIdle()
    }
}
