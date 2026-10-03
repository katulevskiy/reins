package dev.rewarden.android.platform

import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.rewarden.android.RecordingFeedback
import dev.rewarden.android.TestData
import dev.rewarden.android.feedback.Event
import dev.rewarden.android.feedback.FeedbackSettings
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/** The notification channels sound the app's chimes and follow its Sounds & haptics switches. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class NotificationChannelsTest {
    private val context: Context = ApplicationProvider.getApplicationContext()
    private val manager = context.getSystemService(NotificationManager::class.java)
    private var settings = FeedbackSettings()
    private val heard = RecordingFeedback()
    private val notifier = AppNotifier(context, settings = { settings }, feedback = { heard }) {}

    @After
    fun tearDown() {
        Foreground.focused = false
    }

    private fun posted() = shadowOf(manager).allNotifications

    @Test
    fun `approvals chime with the request chime and vibrate like the in-app haptic`() {
        notifier.createChannels()
        notifier.itemPending(TestData.pending("req1"))
        val channel = manager.getNotificationChannel(posted().single().channelId)
        assertEquals("approvals-v${AppNotifier.CHANNEL_VERSION}-sv", channel.id)
        assertTrue(channel.sound.toString().endsWith("/raw/fx_chime_request"))
        assertTrue(channel.shouldVibrate())
        assertEquals(listOf(0L, 18L, 90L, 18L), channel.vibrationPattern.toList())
        assertEquals(NotificationManager.IMPORTANCE_HIGH, channel.importance)
    }

    @Test
    fun `grant reminders and status changes use the attention chime`() {
        notifier.createChannels()
        for (kind in listOf(AppNotifier.Kind.Grants, AppNotifier.Kind.Status)) {
            val channel = manager.getNotificationChannel(notifier.channel(kind))
            assertTrue(kind.name, channel.sound.toString().endsWith("/raw/fx_chime_attention"))
        }
    }

    @Test
    fun `the master off makes notifications silent, and they stay as important`() {
        settings = FeedbackSettings(master = false)
        notifier.itemPending(TestData.pending("req1"))
        val channel = manager.getNotificationChannel(posted().single().channelId)
        assertEquals("approvals-v${AppNotifier.CHANNEL_VERSION}-q", channel.id)
        assertNull(channel.sound)
        assertFalse(channel.shouldVibrate())
        assertEquals(NotificationManager.IMPORTANCE_HIGH, channel.importance)
    }

    @Test
    fun `the request switch and the haptics switch pick the channel each on their own`() {
        settings = FeedbackSettings(requestSounds = false)
        assertEquals("approvals-v${AppNotifier.CHANNEL_VERSION}-v", notifier.channel(AppNotifier.Kind.Approvals))
        settings = FeedbackSettings(haptics = false)
        assertEquals("approvals-v${AppNotifier.CHANNEL_VERSION}-s", notifier.channel(AppNotifier.Kind.Approvals))
        // Alerts are their own switch: turning requests off leaves grant reminders sounding.
        settings = FeedbackSettings(requestSounds = false)
        assertEquals("grants-v${AppNotifier.CHANNEL_VERSION}-sv", notifier.channel(AppNotifier.Kind.Grants))
    }

    @Test
    fun `channels of earlier builds are removed, their sounds cannot be changed`() {
        for (id in listOf("approvals", "grants", "status", "approvals-v1-sv")) {
            manager.createNotificationChannel(NotificationChannel(id, id, NotificationManager.IMPORTANCE_DEFAULT))
        }
        manager.createNotificationChannel(NotificationChannel("updates", "App updates", NotificationManager.IMPORTANCE_DEFAULT))
        notifier.createChannels()
        val ids = manager.notificationChannels.map { it.id }.toSet()
        for (old in listOf("approvals", "grants", "status", "approvals-v1-sv")) assertFalse(old, old in ids)
        assertTrue("updates" in ids) // not ours to touch
        assertTrue("approvals-v${AppNotifier.CHANNEL_VERSION}-sv" in ids)
    }

    @Test
    fun `in front a request chimes in the app and posts nothing`() {
        Foreground.focused = true
        notifier.itemPending(TestData.pending("req1"))
        assertEquals(0, posted().size)
        assertTrue(heard.played(Event.RequestArrived))
    }

    @Test
    fun `losing the approval role in front is heard in the app and posted silently`() {
        Foreground.focused = true
        notifier.deviceReplaced()
        assertTrue(heard.played(Event.Alert))
        assertEquals("status-v${AppNotifier.CHANNEL_VERSION}-q", posted().single().channelId)
    }
}
