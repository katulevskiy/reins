package dev.rewarden.android

import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.rewarden.android.feedback.Cue
import dev.rewarden.android.feedback.Event
import dev.rewarden.android.feedback.Feedback
import dev.rewarden.android.feedback.FeedbackProvider
import dev.rewarden.android.feedback.FeedbackSettings
import dev.rewarden.android.feedback.Haptic
import dev.rewarden.android.platform.AppNotifier
import dev.rewarden.android.platform.AuthResult
import dev.rewarden.android.platform.Foreground
import java.util.concurrent.CopyOnWriteArrayList
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** Everything the app asks the feedback engine for, before any switch or rate limit. */
class RecordingFeedback : Feedback {
    val haptics = CopyOnWriteArrayList<Haptic>()
    val cues = CopyOnWriteArrayList<Cue>()

    override fun haptic(haptic: Haptic) {
        haptics += haptic
    }

    override fun cue(cue: Cue, step: Int) {
        cues += cue
    }

    fun played(event: Event): Boolean = (event.cue == null || event.cue in cues) && (event.haptic == null || event.haptic in haptics)

    fun clear() {
        haptics.clear()
        cues.clear()
    }
}

/** The moments that sound: approving, denying, allowing for a while, revoking, a request arriving; and the settings page. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class FeedbackFlowTest : FlowHarness() {
    private val heard = RecordingFeedback()

    @Before
    fun listen() {
        FeedbackProvider.observer = heard
    }

    @After
    fun stopListening() {
        FeedbackProvider.observer = null
        container.feedbackStore.update { FeedbackSettings() }
    }

    @Test
    fun approvingOnceSoundsLikeSending() {
        openRequest(TestData.searchView())
        heard.clear()
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        assertTrue(heard.played(Event.Approved))
        assertFalse(heard.played(Event.GrantCreated))
    }

    @Test
    fun allowingForAWhileSoundsLikeAPermissionThatStays() {
        openRequest(TestData.searchView())
        tap("moreToggle")
        assertTrue("the section opens with its cue", Cue.Open in heard.cues)
        tap("allMail:HOUR")
        assertTrue(heard.played(Event.Selection))
        heard.clear()
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        assertTrue(heard.played(Event.GrantCreated))
    }

    @Test
    fun denyingAnswersAtOnce() {
        openRequest(TestData.searchView())
        heard.clear()
        tap("deny")
        assertTrue(heard.played(Event.Denied))
        awaitCore { core.denials.isNotEmpty() }
    }

    @Test
    fun noScreenLockIsAnError() {
        authResult = AuthResult.Unavailable
        openRequest(TestData.searchView())
        heard.clear()
        tap("approve")
        awaitText("Set a screen lock", substring = true)
        assertTrue(heard.played(Event.Error))
        assertFalse(heard.played(Event.Approved))
    }

    @Test
    fun revokingAGrantIsHeavy() {
        core.grants = listOf(TestData.grant("g1"))
        core.revokedGrants.clear()
        launch()
        tap("tabGrants")
        tap("grant:g1")
        awaitCore { Cue.Open in heard.cues } // the page opens with its cue
        tap("revoke")
        heard.clear()
        rule.onAllNodes(hasText("Delete")).let { it[it.fetchSemanticsNodes().lastIndex] }.performClick()
        awaitCore { core.revokedGrants.contains("g1") }
        assertTrue(heard.played(Event.Revoked))
    }

    @Test
    fun aRequestArrivingWhileTheAppIsInFrontChimesInsteadOfNotifying() {
        launch()
        awaitTag("openSettings")
        Foreground.focused = true
        heard.clear()
        container.notifier.itemPending(TestData.pending("req7"))
        assertTrue(heard.played(Event.RequestArrived))
        val manager = context.getSystemService(android.app.NotificationManager::class.java)
        assertEquals(0, org.robolectric.Shadows.shadowOf(manager).allNotifications.size)
    }

    @Test
    fun theSoundsPageIsInSettingsAndItsMasterSilencesTheNotificationsToo() {
        launch()
        tap("openSettings")
        awaitText("Sounds and haptics on")
        tap("openSounds")
        awaitTag("soundsMaster")
        heard.clear()
        tap("soundsMaster")
        awaitCore { !container.feedbackStore.current.master }
        // The switch answers in its new state (the engine then stays silent, which the gate tests cover).
        assertTrue(heard.played(Event.ToggleOff))
        assertEquals(AppNotifier.channelId(AppNotifier.Kind.Approvals, sound = false, vibrate = false), container.notifier.channel(AppNotifier.Kind.Approvals))
        tap("soundsMaster")
        awaitCore { container.feedbackStore.current.master }
        assertEquals(AppNotifier.channelId(AppNotifier.Kind.Approvals, sound = true, vibrate = true), container.notifier.channel(AppNotifier.Kind.Approvals))
    }

    @Test
    fun eachKindOfSoundHasItsSwitchAndTheVolumeIsRemembered() {
        launch()
        tap("openSettings")
        tap("openSounds")
        tap("requestSounds")
        awaitCore { !container.feedbackStore.current.requestSounds }
        assertEquals(AppNotifier.channelId(AppNotifier.Kind.Approvals, sound = false, vibrate = true), container.notifier.channel(AppNotifier.Kind.Approvals))
        tap("strength:Strong")
        awaitCore { container.feedbackStore.current.strength == dev.rewarden.android.feedback.HapticStrength.Strong }
        tap("haptics")
        awaitCore { !container.feedbackStore.current.haptics }
        assertEquals(AppNotifier.channelId(AppNotifier.Kind.Approvals, sound = false, vibrate = false), container.notifier.channel(AppNotifier.Kind.Approvals))
    }

    @Test
    fun theServerAddressIsCopied() {
        launch()
        tap("openSettings")
        heard.clear()
        tap("copyServer")
        assertTrue(heard.played(Event.Copied))
        val clipboard = context.getSystemService(android.content.ClipboardManager::class.java)
        assertEquals("http://127.0.0.1:8000", clipboard.primaryClip?.getItemAt(0)?.text?.toString())
    }
}
