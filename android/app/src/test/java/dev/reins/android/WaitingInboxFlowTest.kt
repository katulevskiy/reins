package dev.reins.android

import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performTextInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.platform.Foreground
import dev.reins.core.PendingItem
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** Decisions v2 on the phone: Waiting for you, Later, a denial's note, Why? and answering a group of tickets. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class WaitingInboxFlowTest : FlowHarness() {
    /** Opens the approval screen of [item] (with the search view under it). */
    private fun openApproval(item: PendingItem) {
        core.pending = listOf(item)
        core.approval = TestData.searchView().copy(requestId = item.id)
        launch()
        val row = if (item.ticket != null) "ticket:${item.id}" else "pending:${item.id}"
        awaitTag(row)
        if (!has("approve")) tap(row)
        awaitTag("approve")
    }

    @Test
    fun laterOnTheCardMovesItUnderWaitingForYou() {
        core.pending = listOf(TestData.pending("r1", canDefer = true))
        Foreground.autoPopup = false
        launch()
        tap("later:r1")
        awaitCore { core.laterCalls.contains("r1") }
        awaitTag("ticket:r1")
        assertFalse("off the stack", has("pending:r1"))
        rule.onNodeWithTag("ticketDue:r1", useUnmergedTree = false).assertTextContains("Due in 4 h", substring = true)
    }

    @Test
    fun laterIsOfferedOnlyWhenTheRequestCanWait() {
        core.pending = listOf(TestData.pending("r1"))
        Foreground.autoPopup = false
        launch()
        awaitTag("pending:r1")
        assertFalse(has("later:r1"))
    }

    @Test
    fun laterInTheDockCallsDecideLater() {
        openApproval(TestData.pending("req1", canDefer = true))
        tap("later")
        awaitCore { core.laterCalls.contains("req1") }
        assertEquals(0, prompts.get())
    }

    @Test
    fun denyWithANoteSendsTheNote() {
        openApproval(TestData.pending("req1"))
        tap("denyWithNote")
        awaitTag("denyNoteField")
        rule.onNodeWithTag("denyNoteField").performTextInput("Use the staging bucket")
        tap("denyNoteConfirm")
        awaitCore { core.notedDenials.isNotEmpty() }
        assertEquals("req1" to "Use the staging bucket", core.notedDenials.single())
        assertTrue("not a plain denial", core.denials.isEmpty())
    }

    @Test
    fun anEmptyNoteIsNone() {
        openApproval(TestData.pending("req1"))
        tap("denyWithNote")
        tap("denyNoteConfirm")
        awaitCore { core.notedDenials.isNotEmpty() }
        assertEquals("req1" to null, core.notedDenials.single())
    }

    @Test
    fun denyIsStillOneTap() {
        openApproval(TestData.pending("req1", canDefer = true))
        tap("deny")
        awaitCore { core.denials.contains("req1") }
        assertTrue(core.notedDenials.isEmpty())
    }

    @Test
    fun whyAsksTheAgent() {
        openApproval(TestData.pending("req1", ticket = TestData.ticket("req1")))
        tap("askWhy")
        awaitTag("whyField")
        rule.onNodeWithTag("whyField").performTextInput("Why the production bucket?")
        tap("whySend")
        awaitCore { core.whyQuestions.isNotEmpty() }
        assertEquals("req1" to "Why the production bucket?", core.whyQuestions.single())
    }

    @Test
    fun theInboxShowsTheAgentsAnswerAsTheAgents() {
        core.pending = listOf(
            TestData.pending("t1", ticket = TestData.ticket("t1", left = 3 * 3_600, question = "Why prod?", agentReply = "Staging is down")),
            TestData.pending("t2", label = "Codex", conn = "c2", ticket = TestData.ticket("t2", left = 1_200, stale = "main moved from 1a2b3c4 to 5d6e7f8")),
            TestData.pending("r1"),
        )
        Foreground.autoPopup = false
        launch()
        awaitTag("waitingInbox")
        rule.onNodeWithTag("agentSays:t1").assertTextContains("Agent says", substring = true)
        rule.onNodeWithTag("agentSays:t1").assertTextContains("Staging is down", substring = true)
        assertTrue(has("ticketWhy:t1"))
        assertTrue(has("ticketStale:t2"))
        rule.onNodeWithTag("ticketDue:t1", useUnmergedTree = false).assertTextContains("Due in 3 h", substring = true)
        rule.onNodeWithTag("ticketDue:t2", useUnmergedTree = false).assertTextContains("Due in 20 min", substring = true)
        // The instant request is on the stack; the tickets are not.
        assertTrue(has("pending:r1"))
        assertFalse(has("pending:t1"))
    }

    @Test
    fun aTicketNeverPopsUpAndOpensFromItsRow() {
        core.pending = listOf(TestData.pending("t1", ticket = TestData.ticket("t1")))
        core.approval = TestData.searchView().copy(requestId = "t1")
        launch()
        awaitTag("ticket:t1")
        assertFalse("no sheet by itself", has("approve"))
        tap("ticket:t1")
        awaitTag("approve")
    }

    @Test
    fun allowAllTakesOnlyTheRoutineOnesWithOneCheck() {
        core.pending = listOf(
            TestData.pending("s1", "send", quick = true, ticket = TestData.ticket("s1", left = 600)),
            TestData.pending("s2", "send", quick = true, ticket = TestData.ticket("s2", left = 900)),
            TestData.pending("s3", "send", ticket = TestData.ticket("s3", left = 1_200)),
        )
        Foreground.autoPopup = false
        launch()
        val key = "c1:gmail/send/"
        awaitTag("batchAllow:$key")
        rule.onNodeWithTag("batchAllow:$key").assertTextContains("Allow 2", substring = true)
        tap("batchAllow:$key")
        awaitCore { core.quickApprovals.size == 2 }
        assertEquals(listOf("s1", "s2"), core.quickApprovals.toList().sorted())
        assertEquals(1, prompts.get())
        awaitTag("ticket:s3")
    }

    @Test
    fun denyAllTakesTheWholeGroup() {
        core.pending = listOf(
            TestData.pending("s1", "send", quick = true, ticket = TestData.ticket("s1")),
            TestData.pending("s2", "send", ticket = TestData.ticket("s2")),
        )
        Foreground.autoPopup = false
        launch()
        tap("batchDeny:c1:gmail/send/")
        awaitCore { core.denials.size == 2 }
        assertEquals(0, prompts.get())
        awaitGone("waitingInbox")
    }
}
