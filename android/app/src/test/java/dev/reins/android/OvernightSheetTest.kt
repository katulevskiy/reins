package dev.reins.android

import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.onNodeWithTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.core.ModelState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** The Overnight sheet (Decisions v2, 01 §11): no "too risky" choice any more (it never denies), and "Expires after". */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class OvernightSheetTest : FlowHarness() {
    private fun openSheet() {
        core.model = TestData.modelStatus(ModelState.INSTALLED, size = 412_000_000u)
        launch()
        tap("tabTrust")
        awaitTag("mode:MANUAL")
        tap("forAWhile"); tap("mode:OVERNIGHT")
        awaitTag("overnightSheet")
    }

    @Test
    fun theSheetHasNoTooRiskyRowAndSaysItParks() {
        openSheet()
        assertFalse(has("overnightRisky"))
        assertFalse(showsText("If something is too risky:"))
        awaitText("Auto while you're away. Asks wait for the morning.")
        awaitText("Expires after")
    }

    @Test
    fun expiresAfterShowsTheSettingAndChangesItWithoutAsking() {
        core.overnightExpiry = 24u
        openSheet()
        rule.onNodeWithTag("overnightExpiry:24").assertIsSelected()
        tap("overnightExpiry:6")
        awaitCore { core.expiryCalls.isNotEmpty() }
        assertEquals(listOf(6u), core.expiryCalls.toList())
        assertEquals("a shorter or longer wait never loosens: no owner check", 0, prompts.get())
        assertTrue("a setting, not a start", core.overnightCalls.isEmpty())
        rule.onNodeWithTag("overnightExpiry:6").assertIsSelected()
    }

    @Test
    fun startingStillAsksTheOwner() {
        openSheet()
        hold("confirmOvernight")
        awaitCore { core.overnightCalls.isNotEmpty() }
        assertEquals(1, prompts.get())
        assertTrue(core.expiryCalls.isEmpty())
    }
}
