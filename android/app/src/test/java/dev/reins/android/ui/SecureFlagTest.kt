package dev.rewarden.android.ui

import android.app.Activity
import android.view.WindowManager
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.rewarden.android.ui.common.SecureFlag
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.annotation.Config

/** The mechanism release builds rely on (SECURE_SCREENS is on there); the flow tests run with it off. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class SecureFlagTest {
    private fun Activity.secure() = window.attributes.flags and WindowManager.LayoutParams.FLAG_SECURE != 0

    @Test
    fun flagIsHeldUntilTheLastSecureScreenLeaves() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        assertFalse(activity.secure())

        SecureFlag.acquire(activity)
        assertTrue(activity.secure())
        // A second secure screen arrives before the first one leaves: the flag never drops in between.
        SecureFlag.acquire(activity)
        SecureFlag.release(activity)
        assertTrue(activity.secure())

        SecureFlag.release(activity)
        assertFalse(activity.secure())
    }

    @Test
    fun activitiesAreCountedSeparately() {
        val first = Robolectric.buildActivity(Activity::class.java).setup().get()
        val second = Robolectric.buildActivity(Activity::class.java).setup().get()
        SecureFlag.acquire(first)
        SecureFlag.acquire(second)
        SecureFlag.release(first)
        assertFalse(first.secure())
        assertTrue(second.secure())
        SecureFlag.release(second)
        assertFalse(second.secure())
    }
}
