package dev.rewarden.android.platform

import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.rewarden.android.RecordingFeedback
import dev.rewarden.android.TestData
import dev.rewarden.android.autopilot.AutopilotText
import dev.rewarden.android.feedback.Event
import dev.rewarden.android.feedback.FeedbackSettings
import dev.rewarden.core.AutopilotEvent
import dev.rewarden.core.Verdict
import java.util.concurrent.CopyOnWriteArrayList
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/** What Autopilot posts: quiet grouped decisions with Report, pauses, the ongoing bypass, the download. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class AutopilotNotificationsTest {
    private val context: Context = ApplicationProvider.getApplicationContext()
    private val manager = context.getSystemService(NotificationManager::class.java)
    private var settings = FeedbackSettings()
    private val heard = RecordingFeedback()
    private val events = CopyOnWriteArrayList<AutopilotEvent>()
    private var changes = 0
    private val notifier = AppNotifier(context, settings = { settings }, feedback = { heard }, onAutopilot = { events += it }) { changes++ }

    @After
    fun tearDown() {
        Foreground.focused = false
    }

    private fun posted() = shadowOf(manager).allNotifications

    private fun decisions() = posted().filter { (it.flags and Notification.FLAG_GROUP_SUMMARY) == 0 }

    @Test
    fun `an automatic approval is quiet, grouped, and can be reported`() {
        notifier.createChannels()
        notifier.autoDecided(TestData.autoDecision())
        val n = decisions().single()
        val channel = manager.getNotificationChannel(n.channelId)
        assertTrue(channel.id.startsWith("autopilot-approved-"))
        assertEquals(NotificationManager.IMPORTANCE_LOW, channel.importance)
        assertNull("never heard", channel.sound)
        assertEquals(AppNotifier.GROUP_AUTOPILOT, channel.group)
        assertEquals("Autopilot approved", n.extras.getCharSequence(Notification.EXTRA_TITLE).toString())
        assertEquals("Push to a branch · dkat/rewarden — Claude Code", n.extras.getCharSequence(Notification.EXTRA_TEXT).toString())
        assertEquals(AppNotifier.GROUP_AUTOPILOT, n.group)
        assertEquals(Notification.VISIBILITY_PRIVATE, n.visibility)
        assertEquals("Report", n.actions.single().title.toString())
        val report = shadowOf(n.actions.single().actionIntent).savedIntent
        assertEquals(AppNotifier.ACTION_OPEN_ACTIVITY, report.action)
        assertEquals(42L, report.getLongExtra(AppNotifier.EXTRA_ACTIVITY_ID, -1))
        assertTrue("a summary folds them together", posted().any { (it.flags and Notification.FLAG_GROUP_SUMMARY) != 0 })
        assertEquals(1, changes)
    }

    @Test
    fun `an automatic denial uses its own channel at default importance with a soft sound`() {
        notifier.autoDecided(TestData.autoDecision(verdict = Verdict.DENY))
        val channel = manager.getNotificationChannel(decisions().single().channelId)
        assertTrue(channel.id.startsWith("autopilot-denied-"))
        assertEquals(NotificationManager.IMPORTANCE_DEFAULT, channel.importance)
        assertTrue(channel.sound.toString().endsWith("/raw/fx_close"))
    }

    @Test
    fun `the Autopilot sound switch silences the denial channel`() {
        settings = FeedbackSettings(autopilotSounds = false)
        notifier.autoDecided(TestData.autoDecision(verdict = Verdict.DENY))
        assertNull(manager.getNotificationChannel(decisions().single().channelId).sound)
    }

    @Test
    fun `in front an automatic decision is heard, not posted`() {
        Foreground.focused = true
        notifier.autoDecided(TestData.autoDecision())
        notifier.autoDecided(TestData.autoDecision("req2", Verdict.DENY))
        assertTrue(posted().isEmpty())
        assertTrue(heard.played(Event.AutoApproved))
        assertTrue(heard.played(Event.AutoDenied))
    }

    @Test
    fun `a decision without an activity entry opens the app and has no Report`() {
        notifier.autoDecided(TestData.autoDecision(activityId = null))
        assertTrue(decisions().single().actions.isNullOrEmpty())
    }

    @Test
    fun `mode changes reach the app, a pause is posted, a bypass ending in front is heard`() {
        notifier.autopilotChanged(AutopilotEvent.ModeChanged(null))
        notifier.autopilotChanged(AutopilotEvent.Paused("c1", "Claude Code", "unusual volume"))
        assertEquals(2, events.size)
        val paused = posted().single()
        assertEquals("Autopilot paused for Claude Code", paused.extras.getCharSequence(Notification.EXTRA_TITLE).toString())
        Foreground.focused = true
        notifier.autopilotChanged(AutopilotEvent.BypassEnded(null))
        assertTrue(heard.played(Event.BypassOff))
    }

    @Test
    fun `a bypass shows an ongoing countdown with Stop that ends with it`() {
        val nowMillis = 1_700_000_000_000L
        val notice = AutopilotText.BypassNotice("Bypass on · 15 min left", "Requests from every AI are approved without asking, except the riskiest.", 1_700_000_900, true, emptyList())
        notifier.showBypass(notice, nowMillis)
        val n = posted().single()
        assertTrue((n.flags and Notification.FLAG_ONGOING_EVENT) != 0)
        assertEquals(1_700_000_900_000L, n.`when`)
        assertTrue(n.extras.getBoolean(Notification.EXTRA_CHRONOMETER_COUNT_DOWN))
        assertEquals(900_000L, n.timeoutAfter)
        assertEquals(NotificationManager.IMPORTANCE_LOW, manager.getNotificationChannel(n.channelId).importance)
        val stop = n.actions.single()
        assertEquals("Stop", stop.title.toString())
        assertEquals(BypassStopReceiver.ACTION_STOP, shadowOf(stop.actionIntent).savedIntent.action)
        notifier.showBypass(null)
        assertTrue(posted().isEmpty())
    }

    @Test
    fun `a pending item carries Autopilot's suggestion`() {
        notifier.itemPending(TestData.pending("req1", "read", 3u, suggestion = "Autopilot would approve · 97%"))
        val big = posted().single().extras.getCharSequence(Notification.EXTRA_BIG_TEXT).toString()
        assertTrue(big.endsWith("Autopilot would approve · 97%"))
    }

    @Test
    fun `the download shows progress and a quiet result`() {
        val progress = notifier.downloadNotification(103_000_000, 412_000_000)
        assertEquals(1000, progress.extras.getInt(Notification.EXTRA_PROGRESS_MAX))
        assertEquals(250, progress.extras.getInt(Notification.EXTRA_PROGRESS))
        assertEquals("103 MB of 412 MB", progress.extras.getCharSequence(Notification.EXTRA_TEXT).toString())
        assertTrue(notifier.downloadNotification(5_000_000, 0).extras.getBoolean(Notification.EXTRA_PROGRESS_INDETERMINATE))
        notifier.downloadFinished("The downloaded files did not match.")
        val done = posted().single()
        assertEquals("The model could not be downloaded", done.extras.getCharSequence(Notification.EXTRA_TITLE).toString())
        assertEquals(NotificationManager.IMPORTANCE_LOW, manager.getNotificationChannel(done.channelId).importance)
    }

    @Test
    fun `Autopilot's channels are listed together`() {
        notifier.createChannels()
        for (kind in listOf(AppNotifier.Kind.AutoApproved, AppNotifier.Kind.AutoDenied, AppNotifier.Kind.Bypass, AppNotifier.Kind.ModelDownload)) {
            assertEquals(kind.name, AppNotifier.GROUP_AUTOPILOT, manager.getNotificationChannel(notifier.channel(kind)).group)
        }
        assertEquals("Autopilot", manager.getNotificationChannelGroup(AppNotifier.GROUP_AUTOPILOT).name.toString())
        assertFalse("a channel that never sounds has no silent twin", notifier.channel(AppNotifier.Kind.Bypass).endsWith("-sv"))
    }
}
