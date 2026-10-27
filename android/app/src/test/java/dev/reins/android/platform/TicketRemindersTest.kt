package dev.reins.android.platform

import android.app.AlarmManager
import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.TestData
import dev.reins.android.design.Timers
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class TicketRemindersTest {
    private val context: Context get() = ApplicationProvider.getApplicationContext()
    private val alarms get() = shadowOf(context.getSystemService(AlarmManager::class.java))
    private val manager get() = context.getSystemService(NotificationManager::class.java)
    private val now = 1_700_000_100L

    @Before
    fun setUp() {
        dev.reins.android.TestNativeKeys.install()
        shadowOf(context as android.app.Application).grantPermissions(android.Manifest.permission.POST_NOTIFICATIONS)
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

    /** A ticket whose window is [now - age, now + left]. */
    private fun item(id: String, left: Long, age: Long, quick: Boolean = false) =
        TestData.pending(id, "send", quick = quick, ticket = TestData.ticket(id, left = left, age = age, now = now))

    @Test
    fun `a ticket gets an alarm at half and at nine tenths of its window`() {
        // Window 10 000 s from now - 1 000: reminders at now + 4 000 and now + 8 000.
        TicketReminders.sync(context, listOf(item("t1", left = 9_000, age = 1_000), TestData.pending("instant")), nowSeconds = now)
        val at = alarms.scheduledAlarms.map { it.triggerAtMs }.sorted()
        assertEquals(listOf((now + 4_000) * 1000, (now + 8_000) * 1000), at)
    }

    @Test
    fun `a reminder already due fires at once, and only the latest of those`() {
        // Window 10 000 s, 95 % gone: the 50 % reminder is skipped, the 90 % one fires now.
        TicketReminders.sync(context, listOf(item("t1", left = 500, age = 9_500)), nowSeconds = now)
        assertEquals(listOf((now + 1) * 1000), alarms.scheduledAlarms.map { it.triggerAtMs })
    }

    @Test
    fun `alarms are cancelled when the item is gone`() {
        TicketReminders.sync(context, listOf(item("t1", left = 9_000, age = 1_000), item("t2", left = 9_000, age = 1_000)), nowSeconds = now)
        assertEquals(4, alarms.scheduledAlarms.size)
        TicketReminders.sync(context, listOf(item("t2", left = 9_000, age = 1_000)), nowSeconds = now)
        assertEquals(2, alarms.scheduledAlarms.size)
        TicketReminders.sync(context, emptyList(), nowSeconds = now)
        assertTrue(alarms.scheduledAlarms.isEmpty())
    }

    @Test
    fun `the reminder says what still waits, once, and can be answered`() {
        TicketReminders.sync(context, listOf(item("t1", left = 9_000, age = 1_000, quick = true)), nowSeconds = now)
        TicketReminders.due(context, "t1", 50, now + 4_000)
        val posted = shadowOf(manager).allNotifications
        assertEquals(1, posted.size)
        val n = posted.single()
        assertTrue(n.extras.getCharSequence(Notification.EXTRA_TITLE).toString().startsWith("Still waiting: "))
        assertNotNull("the lock screen shows nothing specific", n.publicVersion)
        assertNotNull("tapping opens it", n.contentIntent)
        assertTrue("a routine one can be denied from here", n.actions.orEmpty().any { it.title == "Deny" })
        assertTrue("a long window left is quiet", n.channelId.startsWith("waiting-"))
        manager.cancelAll()
        TicketReminders.due(context, "t1", 50, now + 4_001)
        assertTrue("never twice", shadowOf(manager).allNotifications.isEmpty())
    }

    @Test
    fun `under an hour left the reminder is on the approvals channel`() {
        TicketReminders.sync(context, listOf(item("t1", left = 1_000, age = 9_000)), nowSeconds = now)
        TicketReminders.due(context, "t1", 90, now + 1)
        assertTrue(shadowOf(manager).allNotifications.single().channelId.startsWith("approvals-"))
    }

    @Test
    fun `no reminder for an item answered since, or while the app is open`() {
        TicketReminders.sync(context, listOf(item("t1", left = 9_000, age = 1_000)), nowSeconds = now)
        TicketReminders.sync(context, emptyList(), nowSeconds = now)
        TicketReminders.due(context, "t1", 50, now + 4_000)
        assertTrue(shadowOf(manager).allNotifications.isEmpty())
        TicketReminders.sync(context, listOf(item("t2", left = 9_000, age = 1_000)), nowSeconds = now)
        Foreground.focused = true
        TicketReminders.due(context, "t2", 50, now + 4_000)
        assertTrue(shadowOf(manager).allNotifications.isEmpty())
    }

    @Test
    fun `the plan is pure`() {
        val plan = TicketReminders.plan(listOf(item("t1", left = 9_000, age = 1_000), TestData.pending("instant")), now)
        assertEquals(listOf(50, 90), plan.map { it.stage })
        assertEquals(listOf(now + 4_000, now + 8_000), plan.map { it.fireAt })
        assertTrue(TicketReminders.plan(listOf(item("gone", left = -10, age = 1_000)), now).isEmpty())
    }

    @Test
    fun `a deferred arrival is quiet unless under an hour is left`() {
        val notifier = AppNotifier(context) {}
        notifier.itemPending(item("t1", left = 3 * 3_600, age = 60))
        notifier.itemPending(item("t2", left = 1_200, age = 60))
        notifier.itemPending(TestData.pending("instant"))
        val byItem = shadowOf(manager).allNotifications.associateBy { it.extras.getString(AppNotifier.EXTRA_ITEM) }
        assertTrue(byItem.getValue("t1").channelId.startsWith("waiting-"))
        assertTrue(byItem.getValue("t2").channelId.startsWith("approvals-"))
        assertTrue(byItem.getValue("instant").channelId.startsWith("approvals-"))
    }

    @Test
    fun `a ticket waiting for the agent gets no reminders, and loses those it had`() {
        val asked = TestData.pending("t1", "send", ticket = TestData.ticket("t1", left = 9_000, age = 1_000, now = now, state = "needs_info", question = "Why?"))
        assertTrue(TicketReminders.plan(listOf(asked), now).isEmpty())
        TicketReminders.sync(context, listOf(item("t1", left = 9_000, age = 1_000)), nowSeconds = now)
        assertEquals(2, alarms.scheduledAlarms.size)
        TicketReminders.sync(context, listOf(asked), nowSeconds = now)
        assertTrue("moved to needs_info: its alarms go", alarms.scheduledAlarms.isEmpty())
        TicketReminders.due(context, "t1", 50, now + 4_000)
        assertTrue(shadowOf(manager).allNotifications.isEmpty())
    }
}
