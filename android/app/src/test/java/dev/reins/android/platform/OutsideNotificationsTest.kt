package dev.reins.android.platform

import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import androidx.compose.ui.graphics.toArgb
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.TestData
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.design.Field
import dev.reins.core.PendingKind
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/** Outside the app (And-Outside 01, 02): asks in their field's colour; a running bypass or Overnight as a Live Update. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class OutsideNotificationsTest {
    private val context: Context = ApplicationProvider.getApplicationContext()
    private val notifier = AppNotifier(context) {}
    private val manager = context.getSystemService(NotificationManager::class.java)

    private fun posted(): List<Notification> = shadowOf(manager).allNotifications

    @After
    fun tearDown() {
        Foreground.focused = false
    }

    @Test
    fun `an ask is colorized in its field`() {
        notifier.createChannels()
        notifier.itemPending(TestData.pending("req1", "read", 3u, quick = true))
        val read = posted().single()
        assertEquals(Field.Read.fill().toArgb(), read.color)
        assertTrue(read.extras.getBoolean(Notification.EXTRA_COLORIZED))
        notifier.itemResolved("req1")
        notifier.itemPending(TestData.pending("req2", "send", 1u, quick = true))
        assertEquals(Field.Push.fill().toArgb(), posted().single().color)
        // What is asked every time is red, as its card on the stack.
        assertEquals(Field.Delete.fill().toArgb(), AppNotifier.notificationColor(TestData.pending("req3", "write", 1u, quick = false)))
    }

    @Test
    fun `pairing and permission asks take their own fields`() {
        assertEquals(Field.Pair.fill().toArgb(), AppNotifier.notificationColor(TestData.pairingItem("p1")))
        assertEquals(Field.Grant.fill().toArgb(), AppNotifier.notificationColor(TestData.pending("g1", "grant", 1u, quick = true)))
        assertEquals(PendingKind.PAIRING, TestData.pairingItem("p1").kind)
    }

    @Test
    fun `a running bypass is red, counts down and asks to be promoted`() {
        val nowMillis = 1_700_000_000_000L
        notifier.showBypass(AutopilotText.BypassNotice("Bypass on · 15 min left", "For every AI.", 1_700_000_900, true, emptyList()), nowMillis)
        val n = posted().single()
        assertEquals(Field.Delete.fill().toArgb(), n.color)
        // A Live Update from Android 16 (API 36; Robolectric runs 35 here, so the flag itself is not set in this test).
        assertTrue(AppNotifier.promotes(36) && !AppNotifier.promotes(35))
        // A promoted notification is coloured, never colorized.
        assertFalse(n.extras.getBoolean(Notification.EXTRA_COLORIZED))
        assertTrue(n.extras.getBoolean(Notification.EXTRA_SHOW_CHRONOMETER))
        assertEquals("Stop", n.actions.single().title.toString())
    }

    @Test
    fun `a running Overnight is violet, the same way`() {
        val nowMillis = 1_700_000_000_000L
        notifier.showOvernight(
            AutopilotText.BypassNotice("Overnight on · 8 h left", "For every AI.", 1_700_028_800, true, emptyList(), mode = dev.reins.core.AutopilotMode.OVERNIGHT),
            nowMillis,
        )
        val n = posted().single()
        assertEquals(Field.Grant.fill().toArgb(), n.color)
        assertFalse(n.extras.getBoolean(Notification.EXTRA_COLORIZED))
        assertTrue(n.extras.getBoolean(Notification.EXTRA_SHOW_CHRONOMETER))
    }
}
