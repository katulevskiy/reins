package dev.reins.android.push

import android.app.NotificationManager
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.FlowHarness
import dev.reins.android.TestData
import dev.reins.android.platform.AppNotifier
import dev.reins.core.CoreException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/** A push is handled while FCM keeps the process awake; WorkManager only retries. Notifications leave with their item. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class InlinePushTest : FlowHarness() {
    private val manager get() = context.getSystemService(NotificationManager::class.java)

    @Test
    fun aPushIsHandledAtOnceWithoutAJob() {
        assertTrue(ReinsMessagingService.handleNow(context, PushPayload("req", "r1")))
        assertEquals(listOf("req:r1"), core.pushes.toList())
    }

    @Test
    fun aPushThatCouldNotBeHandledIsLeftToTheRetryingJob() {
        core.pushFailure = CoreException.Network("offline")
        assertFalse(ReinsMessagingService.handleNow(context, PushPayload("req", "r1")))
    }

    @Test
    fun aNotificationLastsAsLongAsItsRequestAndLeavesOnceItIsGone() {
        container.notifier.createChannels()
        val now = System.currentTimeMillis() / 1000
        // What the core says waits: posting refreshes the list, and a refresh keeps exactly these.
        core.pending = listOf(TestData.pending("r1", createdAt = now - 60), TestData.pending("r2", createdAt = now), TestData.pending("r3", createdAt = now))
        container.notifier.itemPending(TestData.pending("r1", createdAt = now - 60))
        container.notifier.itemPending(TestData.pending("r2", createdAt = now))
        val posted = shadowOf(manager).allNotifications
        assertEquals(2, posted.size)
        val r1 = posted.first { it.extras.getString(AppNotifier.EXTRA_ITEM) == "r1" }
        assertTrue("about nine minutes left: ${r1.timeoutAfter}", r1.timeoutAfter in 530_000L..545_000L)
        // r2 was answered elsewhere: the next refresh takes its notification away, r1 stays.
        container.notifier.dropStale(setOf("r1"), readAt = System.currentTimeMillis() + 1)
        assertEquals(listOf("r1"), shadowOf(manager).allNotifications.map { it.extras.getString(AppNotifier.EXTRA_ITEM) })
        // Something posted after the list was read is not touched.
        container.notifier.itemPending(TestData.pending("r3", createdAt = now))
        container.notifier.dropStale(setOf("r1"), readAt = 0)
        assertEquals(2, shadowOf(manager).allNotifications.size)
    }
}
