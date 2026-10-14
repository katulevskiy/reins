package dev.reins.android.design

import androidx.compose.foundation.layout.Box
import org.robolectric.annotation.GraphicsMode
import androidx.compose.ui.graphics.Color
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.RecordingFeedback
import dev.reins.android.feedback.LocalFeedback
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** A pressable answers every press with the default tap, however often what it draws changes in between. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class PressableTest {
    @get:Rule val rule = createComposeRule()

    /** A press and its release delivered together, before any animation has run, end at rest. */
    @Test
    fun aPressReleasedAtOnceEndsAtRest() {
        val tracker = PressTracker()
        val press = androidx.compose.foundation.interaction.PressInteraction.Press(androidx.compose.ui.geometry.Offset.Zero)
        assertEquals(1f, tracker.update(press))
        assertEquals("the release is never missed", 0f, tracker.update(androidx.compose.foundation.interaction.PressInteraction.Release(press)))
        val again = androidx.compose.foundation.interaction.PressInteraction.Press(androidx.compose.ui.geometry.Offset.Zero)
        val second = androidx.compose.foundation.interaction.PressInteraction.Press(androidx.compose.ui.geometry.Offset.Zero)
        assertEquals(1f, tracker.update(again))
        assertEquals("a second finger changes nothing", null, tracker.update(second))
        assertEquals(null, tracker.update(androidx.compose.foundation.interaction.PressInteraction.Cancel(again)))
        assertEquals(0f, tracker.update(androidx.compose.foundation.interaction.PressInteraction.Release(second)))
    }

    @Test
    fun everyPressTapsWhenThePressRecomposesTheControl() {
        val heard = RecordingFeedback()
        var presses = 0
        rule.setContent {
            CompositionLocalProvider(LocalFeedback provides heard) {
                // Like a tab: the press changes what the control shows, so it composes again with the same action.
                var selected by remember { mutableStateOf(false) }
                val onClick = remember { { presses++; selected = !selected } }
                Box(Modifier.size(48.dp).pressable(dim = if (selected) 0.5f else 0.55f, onClick = onClick).testTag("control"))
            }
        }
        repeat(5) {
            rule.onNodeWithTag("control").performClick()
            rule.waitForIdle()
        }
        assertEquals(5, presses)
        assertEquals(5, heard.defaultTaps.get())
    }
}

/**
 * A quick tap in a scrolling page delivers its press and its release together. The control must come back to rest:
 * it once stayed dimmed ("greyed out", "tinted darker") until the next tap.
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class PressableRestTest {
    @get:Rule val rule = androidx.compose.ui.test.junit4.createAndroidComposeRule<androidx.activity.ComponentActivity>()

    private fun pixel(): Int {
        val view = rule.activity.window.decorView
        val bitmap = android.graphics.Bitmap.createBitmap(view.width, view.height, android.graphics.Bitmap.Config.ARGB_8888)
        view.draw(android.graphics.Canvas(bitmap))
        val bounds = rule.onNodeWithTag("control").fetchSemanticsNode().boundsInWindow
        return bitmap.getPixel(bounds.center.x.toInt(), bounds.center.y.toInt())
    }

    @Test
    fun aPressAndItsReleaseDeliveredTogetherLeaveTheControlAtRest() {
        val source = androidx.compose.foundation.interaction.MutableInteractionSource()
        rule.setContent {
            CompositionLocalProvider(LocalFeedback provides RecordingFeedback()) {
                Column(Modifier.size(300.dp).background(Color.White).verticalScroll(rememberScrollState())) {
                    Box(Modifier.size(120.dp).pressable(dim = 0.3f, interactionSource = source) {}.background(Color.Red).testTag("control"))
                    Box(Modifier.size(600.dp))
                }
            }
        }
        rule.waitForIdle()
        assertEquals("at rest before any tap", android.graphics.Color.RED, pixel())
        repeat(4) {
            // What a quick tap in a scrolling page does: the press waits to see whether it is a scroll, then the press
            // and its release are sent one after the other, before anything else runs.
            rule.runOnIdle {
                val press = androidx.compose.foundation.interaction.PressInteraction.Press(androidx.compose.ui.geometry.Offset(10f, 10f))
                source.tryEmit(press)
                source.tryEmit(androidx.compose.foundation.interaction.PressInteraction.Release(press))
            }
            rule.mainClock.advanceTimeBy(2_000)
            rule.waitForIdle()
            assertEquals("tap ${it + 1}: drawn at full strength again", android.graphics.Color.RED, pixel())
        }
        // Held, it does dim (so the check above can see a stuck press).
        rule.runOnIdle { source.tryEmit(androidx.compose.foundation.interaction.PressInteraction.Press(androidx.compose.ui.geometry.Offset(10f, 10f))) }
        rule.mainClock.advanceTimeBy(2_000)
        rule.waitForIdle()
        org.junit.Assert.assertNotEquals(android.graphics.Color.RED, pixel())
    }
}
