package dev.reins.android.ui

import dev.reins.android.TestData
import dev.reins.android.ui.activity.batchGroups
import dev.reins.android.ui.activity.waitsForMorning
import dev.reins.android.ui.activity.taskProposals
import dev.reins.android.ui.activity.dueIn
import dev.reins.android.ui.activity.inboxByAgent
import dev.reins.android.ui.activity.reminderTimes
import dev.reins.android.ui.activity.waitCards
import dev.reins.android.ui.activity.waitingInbox
import dev.reins.android.ui.activity.WaitCard
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class TicketLogicTest {
    private val now = 1_700_000_100L

    @Test
    fun `the inbox holds only tickets, soonest deadline first`() {
        val pending = listOf(
            TestData.pending("instant", quick = true, canDefer = true),
            TestData.pending("late", ticket = TestData.ticket("late", left = 9 * 3_600)),
            TestData.pending("soon", ticket = TestData.ticket("soon", left = 600)),
            TestData.pending("mid", label = "Codex", conn = "c2", ticket = TestData.ticket("mid", left = 3_600)),
        )
        assertEquals(listOf("soon", "mid", "late"), waitingInbox(pending).map { it.id })
    }

    @Test
    fun `tickets never sit on the stack`() {
        val pending = listOf(
            TestData.pending("a", quick = true),
            TestData.pending("t1", quick = true, ticket = TestData.ticket("t1")),
            TestData.pending("t2", quick = true, ticket = TestData.ticket("t2")),
        )
        val cards = waitCards(pending)
        assertEquals(listOf("a"), cards.map { (it as WaitCard.One).item.id })
    }

    @Test
    fun `the inbox groups by agent in the order of their soonest deadline`() {
        val pending = listOf(
            TestData.pending("c1-late", ticket = TestData.ticket("c1-late", left = 9 * 3_600)),
            TestData.pending("c2-soon", label = "Codex", conn = "c2", ticket = TestData.ticket("c2-soon", left = 600)),
            TestData.pending("c1-mid", ticket = TestData.ticket("c1-mid", left = 3_600)),
        )
        val agents = inboxByAgent(pending)
        assertEquals(listOf("c2", "c1"), agents.map { it.connectionId })
        assertEquals(listOf("c1-mid", "c1-late"), agents[1].items.map { it.id })
        assertEquals("Claude", agents[1].label)
    }

    @Test
    fun `batch groups are one agent's tickets of one kind, two or more`() {
        val pending = listOf(
            TestData.pending("s1", "send", ticket = TestData.ticket("s1", left = 600), quick = true),
            TestData.pending("s2", "send", ticket = TestData.ticket("s2", left = 900), quick = true),
            TestData.pending("s3", "send", ticket = TestData.ticket("s3", left = 1_200)),
            TestData.pending("r1", "read", ticket = TestData.ticket("r1")),
            TestData.pending("x1", "send", label = "Codex", conn = "c2", ticket = TestData.ticket("x1")),
            TestData.pending("instant", "send", quick = true),
        )
        val groups = batchGroups(pending)
        assertEquals(1, groups.size)
        val group = groups.single()
        assertEquals("c1", group.connectionId)
        assertEquals(listOf("s1", "s2", "s3"), group.items.map { it.id })
        assertEquals("Allow never takes what is not routine", listOf("s1", "s2"), group.allowIds)
        assertEquals("Deny takes them all", listOf("s1", "s2", "s3"), group.denyIds)
        val burst = group.burst
        assertEquals(listOf("s1", "s2"), burst.quick)
        assertEquals(listOf("s1", "s2", "s3"), burst.all)
    }

    @Test
    fun `different operations of one service are different kinds`() {
        val pending = listOf(
            TestData.pending("a", "read", service = "telegram", op = "read", ticket = TestData.ticket("a")),
            TestData.pending("b", "read", service = "telegram", op = "send", ticket = TestData.ticket("b")),
        )
        assertTrue(batchGroups(pending).isEmpty())
    }

    @Test
    fun `reminders fall at half and nine tenths of the window`() {
        val t = TestData.ticket("t", left = 6_000, age = 4_000, now = now)
        // Window: [now - 4000, now + 6000], 10 000 s long.
        assertEquals(listOf(now + 1_000, now + 5_000), reminderTimes(t))
    }

    @Test
    fun `the time left reads in words`() {
        assertEquals("Due in 3 h", dueIn(now + 3 * 3_600, now))
        assertEquals("Due in 3 h", dueIn(now + 3 * 3_600 + 1_700, now))
        assertEquals("Due in 45 min", dueIn(now + 45 * 60, now))
        assertEquals("Due in 1 min", dueIn(now + 20, now))
        assertEquals("Due in 2 d", dueIn(now + 2 * 86_400 + 600, now))
        assertEquals("Overdue", dueIn(now, now))
        assertEquals("Overdue", dueIn(now - 10, now))
    }

    @Test
    fun `an item without a ticket has no reminders`() {
        assertNull(TestData.pending("a").ticket)
        assertTrue(waitingInbox(listOf(TestData.pending("a"))).isEmpty())
    }

    @Test
    fun `batch allow skips stale tickets and those waiting for the agent, deny keeps them`() {
        val pending = listOf(
            TestData.pending("ok", "send", quick = true, ticket = TestData.ticket("ok", left = 600)),
            TestData.pending("why", "send", quick = true, ticket = TestData.ticket("why", left = 900, state = "needs_info", question = "Why?")),
            TestData.pending("stale", "send", quick = true, ticket = TestData.ticket("stale", left = 1_200, state = "stale", stale = "main moved")),
            TestData.pending("moved", "send", quick = true, ticket = TestData.ticket("moved", left = 1_500, stale = "main moved")),
        )
        val group = batchGroups(pending).single()
        assertEquals(listOf("ok"), group.allowIds)
        assertEquals(listOf("ok", "why", "stale", "moved"), group.denyIds)
        assertEquals(listOf("ok"), group.burst.quick)
    }

    // ---- Overnight v2 and tasks (Decisions v2 P5) ------------------------------------------------------------------

    @Test
    fun `the inbox groups one agent's requests by task, and Approve all counts only the batchable ones`() {
        val pending = listOf(
            TestData.pending("a1", taskId = "t1", taskTitle = "Fix CI", batchable = true, ticket = TestData.ticket("a1", left = 600)),
            TestData.pending("a2", taskId = "t1", taskTitle = "Fix CI", batchable = true, ticket = TestData.ticket("a2", left = 900)),
            TestData.pending("hard", taskId = "t1", taskTitle = "Fix CI", ticket = TestData.ticket("hard", left = 1_200)),
            TestData.pending("loose", batchable = true, ticket = TestData.ticket("loose", left = 3_600)),
            TestData.pending("other", label = "Codex", conn = "c2", taskId = "t9", ticket = TestData.ticket("other", left = 7_200)),
        )
        val groups = inboxByAgent(pending)
        assertEquals(listOf("c1:t1", "c1", "c2:t9"), groups.map { it.key })
        val task = groups[0]
        assertEquals("t1", task.taskId)
        assertEquals("Fix CI", task.taskTitle)
        assertEquals(listOf("a1", "a2", "hard"), task.items.map { it.id })
        assertEquals("hard ones are answered one by one", listOf("a1", "a2"), task.batchable.map { it.id })
        assertEquals("under no task", null, groups[1].taskId)
        assertEquals(listOf("loose"), groups[1].batchable.map { it.id })
        assertTrue(groups[2].batchable.isEmpty())
    }

    @Test
    fun `kind groups stay within one task`() {
        val pending = listOf(
            TestData.pending("s1", "send", taskId = "t1", ticket = TestData.ticket("s1")),
            TestData.pending("s2", "send", taskId = "t1", ticket = TestData.ticket("s2")),
            TestData.pending("s3", "send", ticket = TestData.ticket("s3")),
        )
        val group = batchGroups(pending).single()
        assertEquals(listOf("s1", "s2"), group.items.map { it.id })
        assertEquals("c1:t1:gmail/send/", group.key)
    }

    @Test
    fun `what Overnight parked waits in the inbox, even before its ticket, and never on the stack`() {
        val morning = now + 6 * 3_600
        val pending = listOf(
            TestData.pending("parked", parkedUntil = morning, ticket = TestData.ticket("parked", left = morning - now + 12 * 3_600)),
            TestData.pending("noTicket", parkedUntil = morning),
            TestData.pending("instant"),
        )
        assertEquals(listOf("noTicket", "parked"), waitingInbox(pending).map { it.id })
        assertTrue(pending[0].waitsForMorning(now))
        assertTrue("the morning came", !pending[0].waitsForMorning(morning))
        assertEquals(listOf("instant"), waitCards(pending).map { (it as WaitCard.One).item.id })
    }

    @Test
    fun `a task proposal is neither in the inbox nor on the stack`() {
        val proposal = TestData.pending("p1", taskProposal = TestData.task(id = ""), ticket = TestData.ticket("p1"))
        assertTrue(waitingInbox(listOf(proposal)).isEmpty())
        assertTrue(waitCards(listOf(proposal)).isEmpty())
        assertEquals(listOf("p1"), taskProposals(listOf(proposal, TestData.pending("r1"))).map { it.id })
    }

    @Test
    fun `a parked ticket reminds only after the morning`() {
        val morning = now + 6 * 3_600
        val t = TestData.ticket("t", left = morning - now + 10_000, age = 0, now = now)
        assertEquals(listOf(morning + 5_000, morning + 9_000), reminderTimes(t, parkedUntil = morning))
    }
}
