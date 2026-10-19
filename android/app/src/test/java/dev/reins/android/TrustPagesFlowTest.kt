package dev.reins.android

import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onNodeWithTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** The Trust pages in Signal: the grant's field and its sheet, the bypass sheet, the profile's glyph icons. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class TrustPagesFlowTest : FlowHarness() {
    @Test
    fun trustRecomposesWhenProfilesArriveAfterOpening() {
        core.apGlobal = FakeCore.ApRow(mode = dev.reins.core.AutopilotMode.AUTO)
        core.model = TestData.modelStatus(dev.reins.core.ModelState.INSTALLED)
        val gate = kotlinx.coroutines.CompletableDeferred<Unit>()
        core.profilesGate = gate
        launch()
        tap("tabTrust")
        awaitTag("mode:AUTO")
        gate.complete(Unit)
        core.profilesGate = null
        tap("profile:personal")
    }

    @Test
    fun trustOpensWithAutoAlreadySelectedAndAnInstalledModel() {
        core.apGlobal = FakeCore.ApRow(mode = dev.reins.core.AutopilotMode.AUTO)
        core.model = TestData.modelStatus(dev.reins.core.ModelState.INSTALLED, updatedAt = 1_700_000_000L)
        launch()
        tap("tabTrust")
        awaitTag("statScored")
        rule.onNodeWithTag("mode:AUTO").assertIsSelected()
        tap("openModel")
    }

    @Test
    fun aGrantIsExtendedFromItsSheetForAChosenLength() {
        core.grants = listOf(TestData.grant("g1"))
        launch()
        tap("tabTrust")
        tap("grant:g1")
        awaitTag("timeLeft")
        tap("extend")
        awaitTag("extendSheet")
        tap("extend:custom")
        tap("extend:MONTH")
        tap("confirmExtend")
        awaitCore { core.extended.isNotEmpty() }
        assertEquals(listOf("g1" to (30uL * 86_400uL)), core.extended.toList())
        assertEquals("extending needs the owner's check", 1, prompts.get())
    }

    @Test
    fun endingAGrantIsHeldNotTapped() {
        core.grants = listOf(TestData.grant("g1"))
        // The fake core is shared between tests: what an earlier one ended is not this one's.
        core.revokedGrants.clear()
        launch()
        tap("tabTrust")
        tap("grant:g1")
        awaitTag("revoke")
        assertTrue(core.revokedGrants.isEmpty())
        // Held through, with no dialog to confirm in.
        hold("revoke")
        assertFalse(showsText("End this grant now?"))
        awaitCore { core.revokedGrants.contains("g1") }
    }

    @Test
    fun theBypassSheetAsksInWordsAndHolds() {
        launch()
        tap("tabTrust")
        tap("mode:BYPASS")
        awaitTag("bypassSheet")
        rule.onNodeWithTag("bypassTitle").assertTextContains("Bypass every AI?")
        rule.onNodeWithTag("bypassLength:60").assertIsSelected()
        assertTrue(showsText("New connections and new phones still ask."))
        tap("bypassLength:custom")
        awaitTag("bypassFor:untilOff")
        assertTrue(showsText("Until I turn it off"))
    }

    @Test
    fun aProfilesIconsAreGlyphsNeverEmoji() {
        launch()
        tap("tabTrust")
        tap("profile:personal")
        tap("renameProfile")
        awaitTag("profileDialog")
        settle()
        val emoji = Regex("[\\x{1F300}-\\x{1FAFF}\\x{2600}-\\x{27BF}]")
        val texts = rule.onAllNodes(hasText("", substring = true), useUnmergedTree = true).fetchSemanticsNodes()
            .flatMap { node -> node.config.getOrElseNullable(androidx.compose.ui.semantics.SemanticsProperties.Text) { null }.orEmpty().map { it.text } }
        assertFalse("no emoji is shown: $texts", texts.any { emoji.containsMatchIn(it) })
    }
}
