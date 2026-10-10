package dev.reins.android

import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.onNodeWithTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.core.PairingView
import dev.reins.core.StartingPolicy
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** The starting rule for new AIs: changed under Grants, said on the pairing sheet; codes are never ticked for the user. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class StartingRuleFlowTest : FlowHarness() {
    @Test
    fun theRuleIsChangedUnderGrants() {
        core.startingPolicy = StartingPolicy.READS_FOR_A_DAY
        launch()
        tap("tabGrants")
        tap("rule:ASK_EVERY_TIME")
        awaitCore { core.startingPolicy == StartingPolicy.ASK_EVERY_TIME }
        tap("rule:READS_FOR_A_DAY")
        awaitCore { core.startingPolicy == StartingPolicy.READS_FOR_A_DAY }
        tap("help:${dev.reins.android.ui.grants.StartingRuleText.HEADER}")
        awaitText("Always asks, whatever you choose", substring = true)
    }

    @Test
    fun thePairingSheetSaysWhatConnectingGives() {
        core.startingPolicy = StartingPolicy.READS_FOR_A_DAY
        core.pending = listOf(TestData.pairingItem("p1"))
        core.pairing = PairingView("p1", "Claude", "claude.ai", byteArrayOf(7, 42, 88), 1_700_000_200, null)
        launch(link("pairing", "p1"))
        awaitTag("startingRuleNote")
        rule.onNodeWithTag("startingRuleNote").assertTextContains("search and read for 24 hours", substring = true)
    }

    @Test
    fun askingEveryTimeAddsNothingToThePairingSheet() {
        core.startingPolicy = StartingPolicy.ASK_EVERY_TIME
        core.pending = listOf(TestData.pairingItem("p1"))
        core.pairing = PairingView("p1", "Claude", "claude.ai", byteArrayOf(7, 42, 88), 1_700_000_200, null)
        launch(link("pairing", "p1"))
        awaitTag("code:42")
        assertFalse(has("startingRuleNote"))
    }

    @Test
    fun anEmailThatLooksLikeACodeIsNeverTickedForTheUser() {
        val view = TestData.searchView().let { v ->
            v.copy(messages = v.messages + TestData.message("m4", "Bank <alerts@bank.com>", sensitive = true))
        }
        openRequest(view)
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        assertEquals(listOf("m1", "m2", "m3"), core.approvals.single().second.selectedMessageIds)
    }
}
