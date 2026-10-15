package dev.reins.android.design

import androidx.compose.foundation.layout.width
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.MotionDurationScale
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
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

/**
 * The hold takes its full 1.2 s even when the system removes animations: the fill runs on elapsed frame time, which
 * the animator duration scale does not touch.
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class HoldButtonTest {
    /** "Remove animations": every animation in this composition would finish at once. */
    @get:Rule val rule = createComposeRule(
        object : MotionDurationScale {
            override val scaleFactor: Float = 0f
        },
    )

    @Test
    fun theHoldIsLinearOverItsDuration() {
        assertEquals(1_200, HOLD_MILLIS)
        assertEquals(180, HOLD_DRAIN_MILLIS)
        assertEquals(0f, holdFraction(0, HOLD_MILLIS))
        assertEquals(0.5f, holdFraction(600_000_000, HOLD_MILLIS), 1e-4f)
        assertEquals(1f, holdFraction(5_000_000_000, HOLD_MILLIS))
    }

    @Test
    fun removingAnimationsNeverShortensTheHold() {
        var confirmed = 0
        rule.setContent {
            CompositionLocalProvider(LocalFeedback provides RecordingFeedback()) {
                HoldButton("Hold to delete", LocalColors.current.danger, Modifier.width(300.dp).testTag("hold")) { confirmed++ }
            }
        }
        rule.mainClock.autoAdvance = false
        rule.onNodeWithTag("hold").performTouchInput { down(center) }
        rule.mainClock.advanceTimeBy(1_000)
        assertEquals("not yet: 1 s of 1.2 s", 0, confirmed)
        rule.mainClock.advanceTimeBy(300)
        assertEquals("held long enough", 1, confirmed)
        rule.onNodeWithTag("hold").performTouchInput { up() }
        rule.mainClock.autoAdvance = true
    }

    @Test
    fun lettingGoEarlyConfirmsNothing() {
        var confirmed = 0
        rule.setContent {
            CompositionLocalProvider(LocalFeedback provides RecordingFeedback()) {
                HoldButton("Hold to delete", LocalColors.current.danger, Modifier.width(300.dp).testTag("hold")) { confirmed++ }
            }
        }
        rule.mainClock.autoAdvance = false
        rule.onNodeWithTag("hold").performTouchInput { down(center) }
        rule.mainClock.advanceTimeBy(700)
        rule.onNodeWithTag("hold").performTouchInput { up() }
        rule.mainClock.advanceTimeBy(2_000)
        assertEquals(0, confirmed)
        rule.mainClock.autoAdvance = true
    }
}
