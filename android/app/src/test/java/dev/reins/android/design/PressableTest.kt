package dev.reins.android.design

import androidx.compose.foundation.layout.Box
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
