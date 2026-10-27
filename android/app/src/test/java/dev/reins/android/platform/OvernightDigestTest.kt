package dev.reins.android.platform

import android.app.AlarmManager
import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.TestData
import dev.reins.android.design.Timers
import dev.reins.core.DigestGroup
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/** Overnight v2 (Decisions v2, 01 §11): parking never notifies; one summary at the morning time, only when something waits. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class OvernightDigestTest {
    private val context: Context get() = ApplicationProvider.getApplicationContext()
    private val alarms get() = shadowOf(context.getSystemService(AlarmManager::class.java))
    private val manager get() = context.getSystemService(NotificationManager::class.java)
    private val now = 1_700_000_100L
    private val morning = now + 7 * 3_600

    @Before
    fun setUp() {
        dev.reins.android.TestNativeKeys.install()
        shadowOf(context as android.app.Application).grantPermissions(android.Manifest.permission.POST_NOTIFICATIONS)
        context.getSharedPreferences("overnight_digest", Context.MODE_PRIVATE).edit().clear().commit()
        context.getSharedPreferences("ticket_reminders", Context.MODE_PRIVATE).edit().clear().commit()
        Timers.frozenNowMillis = now * 1000
        Foreground.focused = false
        AppNotifier(context) {}.createChannels()
    }

    @After
    fun tearDown() {
        Timers.frozenNowMillis = null
        Foreground.focused = false
    }

    private fun digest(at: Long? = morning, total: UInt = 3u) = dev.reins.core.OvernightDigest(
        at = at,
        total = total,
        title = "3 waiting",
        body = "1 from Claude Code (Fix CI), 2 from ChatGPT",
        groups = listOf(DigestGroup("c1", "Claude Code", "t1", "Fix CI", listOf("r1"), 1u)),
    )

    @Test
    fun `the summary is scheduled at the morning time`() {
        OvernightDigest.sync(context, digest(), now)
        assertEquals(listOf(morning * 1000), alarms.scheduledAlarms.map { it.triggerAtMs })
    }

    @Test
    fun `a new digest replaces the earlier alarm, never adds one`() {
        OvernightDigest.sync(context, digest(), now)
        OvernightDigest.sync(context, digest(at = morning + 1_800), now)
        assertEquals(listOf((morning + 1_800) * 1000), alarms.scheduledAlarms.map { it.triggerAtMs })
    }

    @Test
    fun `nothing waiting cancels it, and so does a morning that came`() {
        OvernightDigest.sync(context, digest(), now)
        OvernightDigest.sync(context, null, now)
        assertTrue(alarms.scheduledAlarms.isEmpty())
        OvernightDigest.sync(context, digest(), now)
        OvernightDigest.sync(context, digest(at = null), now)
        assertTrue(alarms.scheduledAlarms.isEmpty())
        OvernightDigest.due(context)
        assertTrue("cancelled: nothing shows", shadowOf(manager).allNotifications.isEmpty())
    }

    @Test
    fun `at the morning it shows once what waits`() {
        OvernightDigest.sync(context, digest(), now)
        OvernightDigest.due(context)
        val n = shadowOf(manager).allNotifications.single()
        assertEquals("3 waiting", n.extras.getCharSequence(Notification.EXTRA_TITLE).toString())
        assertEquals("1 from Claude Code (Fix CI), 2 from ChatGPT", n.extras.getCharSequence(Notification.EXTRA_TEXT).toString())
        assertNotNull("the lock screen shows nothing specific", n.publicVersion)
        assertTrue("quiet", n.channelId.startsWith("waiting-"))
        manager.cancelAll()
        OvernightDigest.due(context)
        assertTrue("never twice", shadowOf(manager).allNotifications.isEmpty())
    }

    @Test
    fun `nothing waiting takes away a summary still shown`() {
        OvernightDigest.sync(context, digest(), now)
        OvernightDigest.due(context)
        assertEquals(1, shadowOf(manager).allNotifications.size)
        OvernightDigest.sync(context, null, now)
        assertTrue(shadowOf(manager).allNotifications.isEmpty())
    }

    @Test
    fun `a plan needs a time and something waiting`() {
        assertNull(OvernightDigest.plan(null))
        assertNull(OvernightDigest.plan(digest(at = null)))
        assertNull(OvernightDigest.plan(digest(total = 0u)))
        assertEquals(morning, OvernightDigest.plan(digest())!!.at)
    }

    @Test
    fun `a parked item does not notify on arrival, and gets no reminder before the morning`() {
        val parked = TestData.pending("p1", parkedUntil = morning, ticket = TestData.ticket("p1", left = morning - now + 12 * 3_600, age = 0, now = now))
        AppNotifier(context) {}.itemPending(parked)
        assertTrue(shadowOf(manager).allNotifications.isEmpty())
        val plan = TicketReminders.plan(listOf(parked), now)
        assertEquals(listOf(50, 90), plan.map { it.stage })
        assertTrue("no reminder before the morning", plan.all { it.fireAt > morning })
        // Once the morning came, it is an ordinary ticket: its arrival would notify (quietly).
        Timers.frozenNowMillis = (morning + 1) * 1000
        AppNotifier(context) {}.itemPending(parked)
        assertEquals(1, shadowOf(manager).allNotifications.size)
    }
}
