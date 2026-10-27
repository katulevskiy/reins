package dev.reins.android

import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.onNodeWithTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.platform.Foreground
import dev.reins.android.ui.activity.TaskField
import dev.reins.android.ui.activity.ceilingChoices
import dev.reins.android.ui.activity.ceilingLabel
import dev.reins.android.ui.activity.lengthChoices
import dev.reins.android.ui.activity.without
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** Tasks on the phone (Decisions v2, 01 §12): a proposal narrowed then approved, Stop, and Approve all for a task. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class TaskCardsFlowTest : FlowHarness() {
    private val now = 1_700_000_100L

    private fun proposal(id: String = "p1") = TestData.pending(
        id, "start a task",
        taskProposal = TestData.task(id = "", reason = "CI is red; I need to push fixes", ceiling = 1u, budget = 20u),
    )

    @Test
    fun removingARepoThenApprovingSendsTheNarrowedTask() {
        core.pending = listOf(proposal())
        launch()
        awaitTag("taskProposal:p1")
        assertFalse("never popped up as a sheet", has("approve"))
        rule.onNodeWithTag("taskReason:p1").assertTextContains("Agent says", substring = true)
        tap("taskChipRemove:Repos:dkat/site")
        awaitGone("taskChip:Repos:dkat/site")
        assertFalse("the last repo stays: removing it would widen the task", has("taskChipRemove:Repos:dkat/reins"))
        tap("taskCeiling:p1:0")
        tap("taskApprove:p1")
        awaitCore { core.approvedTasks.isNotEmpty() }
        val (request, draft) = core.approvedTasks.single()
        assertEquals("p1", request)
        assertEquals(listOf("dkat/reins"), draft.repos)
        assertEquals(listOf("fix/*"), draft.branches)
        assertEquals(0.toUByte(), draft.ceiling)
        assertEquals("the owner was asked", 1, prompts.get())
        assertTrue(core.taskOwners.single() is dev.reins.android.platform.OwnerPrompt)
        awaitGone("taskProposal:p1")
    }

    @Test
    fun refusedOwnerCheckApprovesNothing() {
        core.pending = listOf(proposal())
        authResult = dev.reins.android.platform.AuthResult.Cancelled
        launch()
        tap("taskApprove:p1")
        awaitCore { prompts.get() == 1 }
        settle()
        assertTrue(core.approvedTasks.isEmpty())
        awaitTag("taskProposal:p1")
    }

    @Test
    fun denyIsTheNormalDeny() {
        core.pending = listOf(proposal())
        launch()
        tap("taskDeny:p1")
        awaitCore { core.denials.contains("p1") }
        assertTrue(core.approvedTasks.isEmpty())
        assertEquals(0, prompts.get())
    }

    @Test
    fun aLiveTaskStopsWithStopAndThenReadsAsEnded() {
        core.tasks = listOf(TestData.task(id = "task1", title = "Fix CI", startedAt = now - 600, auto = 12u, asked = 2u, parked = 1u))
        Foreground.autoPopup = false
        launch()
        awaitTag("task:task1")
        tap("taskStop:task1")
        awaitCore { core.endedTasks.contains("task1") }
        assertEquals("Stop is never asked", 0, prompts.get())
        awaitTag("taskEnded:task1")
        rule.onNodeWithTag("taskEnded:task1").assertTextContains("Fix CI ended · 12 auto · 2 asked · 1 parked")
        assertFalse(has("task:task1"))
    }

    @Test
    fun approveAllForATaskTakesOnlyTheBatchableOnes() {
        core.pending = listOf(
            TestData.pending("a1", taskId = "t1", taskTitle = "Fix CI", batchable = true, ticket = TestData.ticket("a1", left = 600)),
            TestData.pending("a2", taskId = "t1", taskTitle = "Fix CI", batchable = true, ticket = TestData.ticket("a2", left = 900)),
            TestData.pending("hard", taskId = "t1", taskTitle = "Fix CI", ticket = TestData.ticket("hard", left = 1_200)),
        )
        Foreground.autoPopup = false
        launch()
        awaitTag("ticketTask:c1:t1")
        rule.onNodeWithTag("approveAll:c1:t1").assertTextContains("Approve all 2", substring = true)
        tap("approveAll:c1:t1")
        awaitCore { core.approveAllCalls.isNotEmpty() }
        assertEquals(listOf("c1" to "t1"), core.approveAllCalls.toList())
        assertEquals(1, prompts.get())
        awaitTag("ticket:hard")
        awaitGone("ticket:a1")
        assertFalse("only hard ones left: no Approve all", has("approveAll:c1:t1"))
    }

    @Test
    fun aParkedItemShowsParkedUnderWaitingForYou() {
        core.pending = listOf(TestData.pending("p", parkedUntil = now + 6 * 3_600))
        launch()
        awaitTag("ticketParked:p")
        assertFalse("never on the stack", has("pending:p"))
        assertFalse("never pops up", has("approve"))
    }

    @Test
    fun narrowingOnlyNarrows() {
        val t = TestData.task(repos = listOf("a/b"), people = listOf("bob@x.io"), operations = listOf("git_push"))
        assertEquals("the last repo stays", t, t.without(TaskField.Repos, "a/b"))
        assertEquals("the last kind stays", t, t.without(TaskField.Kinds, "git_push"))
        assertEquals("people can go to nobody", emptyList<String>(), t.without(TaskField.People, "bob@x.io").people)
        assertEquals(listOf<UByte>(0u, 1u), ceilingChoices(1u))
        assertEquals("never L3", listOf<UByte>(0u, 1u, 2u), ceilingChoices(3u))
        assertEquals("Recoverable", ceilingLabel(1u))
        assertEquals(listOf(900L, 3_600L, 7_200L), lengthChoices(7_200))
    }
}
