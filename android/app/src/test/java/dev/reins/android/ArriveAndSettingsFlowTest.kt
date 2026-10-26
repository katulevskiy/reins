package dev.reins.android

import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.onNodeWithTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.ui.signin.SetupPage
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** Arrive and Settings in Signal: the kite, the five-page tour, the sounds to feel, the sign-out sheet. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class ArriveAndSettingsFlowTest : FlowHarness() {
    @Test
    fun theWelcomeShowsTheArtAndWhatReinsDoes() {
        core.session = null
        launch()
        awaitTag("kite")
        // Next (Arrive 01): the art behind a frosted pane, the two lines on it.
        assertTrue(has("welcomeHeadline"))
        assertTrue(has("continue"))
    }

    @Test
    fun theTourHasFivePages() {
        assertEquals(5, SetupPage.entries.size)
        launch()
        tap("tabSettings")
        tap("takeTour")
        awaitTag("setupWelcome")
        assertTrue(showsText("Nothing happens behind your back"))
    }

    @Test
    fun soundsAreFeltFromChipsAndTheStrengthIsAChoice() {
        launch()
        tap("tabSettings")
        tap("openSounds")
        awaitTag("strength:Strong")
        tap("strength:Strong")
        rule.onNodeWithTag("strength:Strong").assertIsSelected()
        tap("try:Approve")
        assertTrue(showsText("Feel them", substring = true) || showsText("FEEL THEM"))
    }
}
