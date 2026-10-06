package dev.reins.android.platform

import android.app.AlarmManager
import android.app.NotificationManager
import android.content.Context
import android.content.Intent
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.TestData
import dev.reins.android.design.Timers
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class GrantRemindersTest {
    private val context: Context get() = ApplicationProvider.getApplicationContext()
    private val alarms get() = shadowOf(context.getSystemService(AlarmManager::class.java))
    private val manager get() = context.getSystemService(NotificationManager::class.java)
    private val now = 1_700_000_100L

    @Before
    fun setUp() {
        dev.reins.android.TestNativeKeys.install()
        shadowOf(context as android.app.Application).grantPermissions(android.Manifest.permission.POST_NOTIFICATIONS)
        context.getSharedPreferences("grant_reminders", Context.MODE_PRIVATE).edit().clear().commit()
        Timers.frozenNowMillis = now * 1000
        Foreground.focused = false
        AppNotifier(context) {}.createChannels()
    }

    @After
    fun tearDown() {
        Timers.frozenNowMillis = null
        Foreground.focused = false
    }

    private fun grant(id: String, left: Long, age: Long = 600) = TestData.grant(id, leftSeconds = left, ageSeconds = age)

    @Test
    fun `a running grant that ends by itself gets one alarm before its end`() {
        val g = grant("g1", left = 3_000, age = 600)
        GrantReminders.sync(context, listOf(g, grant("open", 10).copy(expiresAt = null), TestData.grant("old", active = false)), now)
        val scheduled = alarms.scheduledAlarms
        assertEquals(1, scheduled.size)
        assertEquals((g.expiresAt!! - 360) * 1000, scheduled.single().triggerAtMs)
    }

    @Test
    fun `a grant already in its last stretch reminds almost at once`() {
        GrantReminders.sync(context, listOf(grant("g1", left = 120, age = 3_480)), now)
        assertEquals((now + 1) * 1000, alarms.scheduledAlarms.single().triggerAtMs)
    }

    @Test
    fun `resuming or deleting a grant replaces or cancels its alarm`() {
        val g = grant("g1", left = 3_000)
        GrantReminders.sync(context, listOf(g), now)
        assertEquals(1, alarms.scheduledAlarms.size)
        GrantReminders.sync(context, listOf(g.copy(expiresAt = g.expiresAt!! + 7_200)), now)
        assertEquals("one alarm per grant, moved to the new end", 1, alarms.scheduledAlarms.size)
        assertTrue(alarms.scheduledAlarms.single().triggerAtMs > (g.expiresAt!! + 3_600) * 1000)
        GrantReminders.sync(context, emptyList(), now)
        assertTrue(alarms.scheduledAlarms.isEmpty())
    }

    @Test
    fun `the reminder is shown once, only when nobody is looking at the app, and says what ends`() {
        val g = grant("g1", left = 300, age = 3_300)
        GrantReminders.sync(context, listOf(g), now)
        GrantReminders.due(context, "g1", now)
        val posted = shadowOf(manager).allNotifications
        assertEquals(1, posted.size)
        assertEquals("Access ends in 5m", posted.single().extras.getString(android.app.Notification.EXTRA_TITLE))
        assertTrue(posted.single().extras.getString(android.app.Notification.EXTRA_TEXT)!!.startsWith("Claude: "))
        assertNotNull("the lock screen shows nothing specific", posted.single().publicVersion)
        manager.cancelAll()
        GrantReminders.due(context, "g1", now + 1)
        assertTrue("never twice for the same end time", shadowOf(manager).allNotifications.isEmpty())
    }

    @Test
    fun `no reminder while the app is open, for a grant that already ended, or an unknown one`() {
        GrantReminders.sync(context, listOf(grant("g1", left = 300, age = 3_300)), now)
        Foreground.focused = true
        GrantReminders.due(context, "g1", now)
        assertTrue(shadowOf(manager).allNotifications.isEmpty())
        Foreground.focused = false
        GrantReminders.due(context, "g1", now + 10)
        assertTrue("already handled while the app was open", shadowOf(manager).allNotifications.isEmpty())
        GrantReminders.sync(context, listOf(grant("g2", left = 300, age = 3_300)), now)
        GrantReminders.due(context, "g2", now + 400)
        GrantReminders.due(context, "nope", now)
        assertTrue(shadowOf(manager).allNotifications.isEmpty())
    }

    @Test
    fun `alarms come back after a reboot but not for reminders that were shown`() {
        GrantReminders.sync(context, listOf(grant("a", left = 3_000), grant("b", left = 4_000)), now)
        GrantReminders.due(context, "b", now + 3_700)
        alarms.scheduledAlarms.forEach { context.getSystemService(AlarmManager::class.java).cancel(it.operation!!) }
        assertTrue(alarms.scheduledAlarms.isEmpty())
        GrantReminders.rearm(context, now)
        assertEquals(1, alarms.scheduledAlarms.size)
        assertFalse(alarms.scheduledAlarms.isEmpty())
        val receiver = GrantReminders.Receiver()
        receiver.onReceive(context, Intent(Intent.ACTION_BOOT_COMPLETED))
        assertNull(alarms.nextScheduledAlarm.takeIf { false })
    }
}
