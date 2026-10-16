package dev.reins.android

import android.os.Looper
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.work.ListenableWorker
import androidx.work.testing.TestListenableWorkerBuilder
import dev.reins.android.push.RegisterDeviceWorker
import dev.reins.android.state.SessionState
import dev.reins.android.ui.signin.UnlockViewModel
import dev.reins.core.CoreException
import java.time.Duration
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * A core `session()` that throws (the store is busy, or the read failed) is tried again. It is not "signed out":
 * the app keeps what it was showing and does not crash.
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class SessionFailureTest : FlowHarness() {
    private val busy = CoreException.Storage("account data is busy in another part of the app; try again")

    @Test
    fun aBusySessionIsKeptAndARealSignOutStillSignsOut() = runBlocking {
        val info = core.session!!
        container.refreshSession()
        assertEquals(SessionState.SignedIn(info), container.state.session.value)
        val pending = listOf(TestData.pending())
        container.state.setPending(pending)
        container.state.setSyncNotice("kept")

        core.sessionError = busy
        container.refreshSession()
        assertEquals(SessionState.SignedIn(info), container.state.session.value)
        assertEquals(pending, container.state.pending.value)
        assertEquals("kept", container.state.syncNotice.value)

        try {
            container.registerDevice(force = false)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            // The failed read may be reported. It must not be handled as a sign-out.
        }
        assertEquals(SessionState.SignedIn(info), container.state.session.value)
        assertEquals(pending, container.state.pending.value)

        core.sessionError = null
        core.sessionThrows = 0
        core.session = null
        container.refreshSession()
        assertEquals(SessionState.SignedOut, container.state.session.value)
    }

    @Test
    fun aSessionThatFailsTwiceThenAnswersSignsIn() = runBlocking {
        val info = core.session!!
        container.state.setSession(SessionState.SignedOut)
        core.sessionError = busy
        core.sessionThrows = 2
        container.refreshSession()
        assertEquals(SessionState.SignedIn(info), container.state.session.value)
        assertTrue(container.state.sessionChecked.value)
    }

    @Test
    fun aFailedSyncNoticeLeavesTheBannerUp() = runBlocking {
        container.state.setSyncNotice("kept")
        core.syncNoticeError = busy
        container.refreshSyncNotice()
        assertEquals("kept", container.state.syncNotice.value)
        core.syncNoticeError = null
        core.syncNotice = "merged"
        container.refreshSyncNotice()
        assertEquals("merged", container.state.syncNotice.value)
    }

    @Test
    fun aBusySessionRetriesBackgroundRegistration() = runBlocking {
        val info = core.session!!
        container.refreshSession()
        core.sessionError = busy
        val result = TestListenableWorkerBuilder<RegisterDeviceWorker>(context).build().doWork()
        assertEquals(ListenableWorker.Result.retry(), result)
        assertEquals(SessionState.SignedIn(info), container.state.session.value)
    }

    @Test
    fun unlockingDoesNotSignOutWhenTheSessionCannotBeRead() {
        val info = core.session!!
        runBlocking { container.refreshSession() }
        core.sessionError = busy
        val open = UnlockViewModel(container, "Pixel")
        open.unlock(FakeCore.RECOVERY_CODE)
        untilSettled { !open.ui.value.busy }
        assertFalse(open.ui.value.busy)
        assertNotNull(open.ui.value.error)
        assertEquals(SessionState.SignedIn(info), container.state.session.value)

        runBlocking {
            container.state.setSession(SessionState.Loading)
            container.setKeysLocked(true)
        }
        core.sessionError = busy
        core.sessionThrows = 0
        val locked = UnlockViewModel(container, "Pixel")
        locked.unlock(FakeCore.RECOVERY_CODE)
        untilSettled { !locked.ui.value.busy }
        assertFalse(locked.ui.value.busy)
        assertEquals(SessionState.Loading, container.state.session.value)
        assertTrue(container.state.keysLocked.value)
    }

    /** Advances both the compose clock and the main looper, so a retry wait can finish either way. */
    private fun untilSettled(done: () -> Boolean) {
        val deadline = System.currentTimeMillis() + 8_000
        while (!done() && System.currentTimeMillis() < deadline) {
            rule.mainClock.advanceTimeBy(400)
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(400))
            Thread.sleep(30)
        }
    }
}
