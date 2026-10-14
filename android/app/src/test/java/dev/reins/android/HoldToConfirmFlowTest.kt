package dev.reins.android

import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTouchInput
import dev.reins.android.design.holdTickGap
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.FeedbackProvider
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** Hold to confirm: Disconnect only happens once the button was held long enough; letting go early changes nothing. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class HoldToConfirmFlowTest : FlowHarness() {
    private val heard = RecordingFeedback()

    @Before
    fun listen() {
        FeedbackProvider.observer = heard
        core.revokedConnections.clear()
    }

    @After
    fun stopListening() {
        FeedbackProvider.observer = null
    }

    private fun openConnection() {
        launch()
        tap("tabSettings")
        tap("connection:c1")
        awaitTag("disconnect")
        rule.onNodeWithTag("disconnect").performScrollTo()
    }

    private fun hold(millis: Long) {
        rule.mainClock.autoAdvance = false
        rule.onNodeWithTag("disconnect").performTouchInput { down(center) }
        rule.mainClock.advanceTimeBy(millis)
        // Held long enough, the page has gone back by the time the finger lifts.
        runCatching { rule.onNodeWithTag("disconnect").performTouchInput { up() } }
        rule.mainClock.advanceTimeBy(400)
        rule.mainClock.autoAdvance = true
        rule.waitForIdle()
    }

    @Test
    fun lettingGoEarlyDisconnectsNothing() {
        openConnection()
        heard.clear()
        hold(800)
        assertTrue(core.revokedConnections.isEmpty())
        assertTrue("a soft release cue", heard.played(Event.HoldReleased))
        assertTrue("ticks while held", heard.played(Event.HoldTick))
    }

    @Test
    fun holdingLongEnoughDisconnects() {
        openConnection()
        heard.clear()
        hold(2_300)
        awaitCore { core.revokedConnections.isNotEmpty() }
        assertEquals(listOf("c1"), core.revokedConnections.toList())
        assertTrue("a heavy thump and the success sound", heard.played(Event.HoldConfirmed))
    }

    @Test
    fun theTicksSpeedUpAsTheHoldFills() {
        assertEquals(260L, holdTickGap(0f))
        assertEquals(70L, holdTickGap(1f))
        assertTrue(holdTickGap(0.5f) in 71L..259L)
    }
}
