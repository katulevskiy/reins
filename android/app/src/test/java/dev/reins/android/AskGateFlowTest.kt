package dev.reins.android

import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** An ask on screen that a bypass, Lockdown or Autopilot then decides closes by itself and does not come back. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class AskGateFlowTest : FlowHarness() {
    @Test
    fun anAskAModeDecidesClosesByItselfAndStaysGone() {
        openRequest(TestData.searchView())
        assertTrue(has("sheet"))
        // The core's list still names it (read a moment before the bypass approved it): it must not come back.
        container.notifier.autoDecided(TestData.autoDecision(id = "req1", decidedBy = "bypass"))
        awaitGone("sheet")
        awaitGone("pending:req1")
        // A refresh that still reads it (off the main thread, which the refresh needs).
        val refreshed = java.util.concurrent.CountDownLatch(1)
        Thread {
            kotlinx.coroutines.runBlocking { container.refreshPending() }
            refreshed.countDown()
        }.start()
        awaitCore { refreshed.count == 0L }
        assertFalse("not offered again", has("sheet"))
        assertFalse(has("pending:req1"))
        assertTrue("never answered by the owner", core.approvals.isEmpty() && core.denials.isEmpty())
    }
}
