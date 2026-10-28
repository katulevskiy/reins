package dev.reins.android

import androidx.compose.ui.test.*
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.core.ActivityHold
import dev.reins.core.HoldState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** A held email on its Activity row (08 R10): Undo while it waits, "Still send?" when it went too late; the setting. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class SendHoldFlowTest : FlowHarness() {
    /** The harness's frozen clock, in seconds. */
    private val now = 1_700_000_100L
    private val send = TestData.label("gmail", "send", "Send an email to", "friend@example.com")

    /** The hold's line sits inside its row's merged semantics: looked up in the unmerged tree. */
    private fun shown(tag: String): Boolean {
        settle()
        return rule.onAllNodes(hasTestTag(tag), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
    }

    private fun awaitShown(tag: String) = rule.waitUntil(10_000) { shown(tag) }

    private fun held(state: HoldState, releaseAt: Long, outcome: String = if (state == HoldState.LATE) "not_sent" else "held") {
        core.activity = listOf(TestData.entry(1, "send", outcome, 1u, at = now - 5, label = send, decidedBy = "autopilot", hold = ActivityHold("h1", releaseAt, state)))
    }

    @Test
    fun aWaitingSendCountsDownAndUndoes() {
        held(HoldState.WAITING, now + 23)
        launch(); awaitShown("hold:h1")
        rule.onNodeWithTag("holdText:h1", useUnmergedTree = true).assertTextEquals("Sending in 23 s")
        rule.onNodeWithTag("outcome:1", useUnmergedTree = true).assertExists()
        assertTrue(!shown("holdSend:h1"))
        tap("holdUndo:h1")
        awaitCore { core.holdCalls.toList() == listOf("undo:h1") }
        // Undo is no approval: nothing asked the owner.
        assertEquals(0, prompts.get())
    }

    @Test
    fun aSendWhoseCountdownRanOutIsReleased() {
        held(HoldState.WAITING, now - 1)
        launch(); awaitShown("hold:h1")
        rule.onNodeWithTag("holdText:h1", useUnmergedTree = true).assertTextEquals("Sending")
        assertTrue(!shown("holdUndo:h1"))
        awaitCore { core.releaseHoldsCalls.get() >= 1 }
    }

    @Test
    fun aLateSendAsksAndSendsAfterTheOwnerCheck() {
        held(HoldState.LATE, now - 1_000)
        launch(); awaitShown("hold:h1")
        rule.onNodeWithTag("holdText:h1", useUnmergedTree = true).assertTextEquals("Still send?")
        rule.onNodeWithTag("outcome:1", useUnmergedTree = true).assertExists()
        assertTrue(rule.onAllNodes(hasText("Not sent"), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty())
        tap("holdSend:h1")
        awaitCore { core.holdCalls.toList() == listOf("send:h1") }
        assertEquals(1, prompts.get())
    }

    @Test
    fun aLateSendCanBeDiscarded() {
        held(HoldState.LATE, now - 1_000)
        launch(); awaitShown("hold:h1")
        tap("holdDiscard:h1")
        awaitCore { core.holdCalls.toList() == listOf("undo:h1") }
    }

    @Test
    fun aSendingSendHasNoButtons() {
        held(HoldState.SENDING, now - 1)
        launch(); awaitShown("hold:h1")
        rule.onNodeWithTag("holdText:h1", useUnmergedTree = true).assertTextEquals("Sending")
        assertTrue(!shown("holdUndo:h1") && !shown("holdSend:h1") && !shown("holdDiscard:h1"))
    }

    @Test
    fun theSendHoldIsSetOnTrust() {
        core.model = TestData.modelStatus(dev.reins.core.ModelState.INSTALLED, size = 412_000_000u)
        launch()
        tap("tabTrust")
        awaitTag("sendHold")
        // What it does is only behind the (?).
        assertTrue(!showsText("Approved emails wait", substring = true))
        tap("sendHold:0")
        awaitCore { core.sendHoldCalls.toList() == listOf(0u) }
        tap("sendHold:120")
        awaitCore { core.sendHoldCalls.toList() == listOf(0u, 120u) }
        tap("help:Send hold")
        awaitText("Approved emails wait", substring = true)
    }
}
