package dev.reins.android.push

import android.app.NotificationManager
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.FlowHarness
import dev.reins.android.profiles.WorkProfiles
import dev.reins.core.HoldsReleased
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/** A held email's push (08 R10) is silent: it releases what is due; only a send that now asks "Still send?" notifies. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class HoldPushTest : FlowHarness() {
    private val manager get() = context.getSystemService(NotificationManager::class.java)

    @Test
    fun aHoldPushIsAccepted() {
        assertEquals(PushPayload("hold", "h-1a2B"), PushPayload.parse(mapOf("t" to "hold", "id" to "h-1a2B")))
        assertEquals(null, PushPayload.parse(mapOf("t" to "hold", "id" to "../x")))
    }

    @Test
    fun aHoldPushReleasesWhatIsDueAndNotifiesNothing() {
        container.notifier.createChannels()
        core.holdsReleased = HoldsReleased(sent = 2u, late = 0u)
        assertTrue(ReinsMessagingService.handleNow(context, PushPayload("hold", "h1")))
        assertEquals(1, core.releaseHoldsCalls.get())
        // Released, not handed to the core's generic push handling.
        assertTrue(core.pushes.isEmpty())
        assertTrue(shadowOf(manager).allNotifications.isEmpty())
    }

    @Test
    fun aSendThatWentTooLateAsksOnce() {
        container.notifier.createChannels()
        core.holdsReleased = HoldsReleased(sent = 0u, late = 2u)
        // What Activity reads next: the sends that ask (the refresh after the push keeps their notification).
        core.activity = listOf(1L, 2L).map {
            dev.reins.android.TestData.entry(it, "send", "not_sent", 1u, hold = dev.reins.core.ActivityHold("h$it", 1_700_000_000, dev.reins.core.HoldState.LATE))
        }
        assertTrue(ReinsMessagingService.handleNow(context, PushPayload("hold", "h1")))
        val posted = shadowOf(manager).allNotifications
        assertEquals(1, posted.size)
        assertEquals("Still send?", posted.single().extras.getCharSequence(android.app.Notification.EXTRA_TITLE).toString())
        assertEquals("2 emails were not sent", posted.single().extras.getCharSequence(android.app.Notification.EXTRA_TEXT).toString())
    }

    @Test
    fun aHoldPushThatFailsIsRetried() {
        core.pushFailure = dev.reins.core.CoreException.Network("offline")
        assertEquals(false, ReinsMessagingService.handleNow(context, PushPayload("hold", "h1")))
    }

    @Test
    fun aHoldPushForAnotherProfileStaysSilent() = runBlocking {
        var elsewhere = 0
        var asked = 0
        val handler = PushHandler(
            core,
            onReplaced = {},
            ownerOf = { PushOwner.Elsewhere(WorkProfiles.PERSONAL) },
            onElsewhere = { elsewhere++ },
            onStillSend = { asked++ },
        )
        assertEquals(PushOutcome.DONE, handler.handle(PushPayload("hold", "h1", "0123456789abcdef")))
        assertEquals(0, elsewhere)
        assertEquals(0, asked)
        assertEquals(0, core.releaseHoldsCalls.get())
        // Any other push for another profile still says "Open Reins".
        handler.handle(PushPayload("req", "r1", "0123456789abcdef"))
        assertEquals(1, elsewhere)
    }
}
