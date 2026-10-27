package dev.reins.android

import android.os.Looper
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.core.PhoneRole
import java.time.Duration
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * A phone that views the account (phone roles) runs the same foreground loop as the approving phone: the core's `sync`
 * follows the account's log on it (never the approval-only poll), and each turn reads the lists and the modes again,
 * so a bypass the primary starts shows here, with its end, without opening anything.
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class ViewerSyncFlowTest : FlowHarness() {
    private fun turns(n: Int) = repeat(n) {
        shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(300))
        settle()
    }

    private fun bypassOnThePrimary() {
        val now = System.currentTimeMillis() / 1000
        core.apGlobal = FakeCore.ApRow(bypassUntil = now + 3_600, bypassSince = now)
    }

    @Test
    fun aViewerShowsThePrimarysBypassWithItsNextTurn() {
        core.session = dev.reins.core.SessionInfo("http://127.0.0.1:8000", "me@example.com")
        core.setupRole = PhoneRole.VIEWER
        launch()
        awaitCore { core.syncStarts.get() > 0 }
        bypassOnThePrimary()
        val started = core.syncStarts.get()
        awaitCore {
            turns(1)
            core.syncStarts.get() > started && container.state.autopilot.value?.bypassUntil != null
        }
    }
}
