package dev.reins.android.ui.activity

import dev.reins.core.PendingItem
import dev.reins.core.TicketView

// Waiting for you (Decisions v2, 03 §5.2): requests that became tickets. The agent carries on and collects the outcome
// later, so they wait in their own section above Today, soonest deadline first, by agent; never on the stack.

/** The tickets that wait for the user, soonest deadline first. */
fun waitingInbox(pending: List<PendingItem>): List<PendingItem> =
    pending.filter { it.ticket != null }.sortedWith(compareBy<PendingItem> { it.ticket!!.deadline }.thenBy { it.createdAt })

/** One agent's tickets, soonest deadline first. */
data class AgentTickets(val connectionId: String, val label: String, val items: List<PendingItem>)

/** The inbox by agent, the agent with the soonest deadline first. */
fun inboxByAgent(pending: List<PendingItem>): List<AgentTickets> =
    waitingInbox(pending)
        .groupBy { it.connectionId }
        .map { (id, items) -> AgentTickets(id, items.first().connectionLabel, items) }

/**
 * One agent's tickets of one kind (the same service, action and operation), answered together. Allow takes only the
 * routine ones ([PendingItem.quick]) that are settled ([settled]): it never crosses what has to be looked at one by
 * one. Deny takes them all.
 */
data class TicketGroup(val connectionId: String, val label: String, val kind: String, val items: List<PendingItem>) {
    val allowIds: List<String> get() = items.filter { it.quick && settled(it) }.map { it.id }
    val denyIds: List<String> get() = items.map { it.id }

    /** The group as the stack's burst, for [answerBurst]: one owner's check, then each request on its own. */
    val burst: Burst get() = Burst(connectionId, label, allowIds, denyIds)

    /** For tags: the agent and the kind. */
    val key: String get() = "$connectionId:$kind"
}

/**
 * Neither stale (the world moved since it was asked or approved: it is looked at again) nor waiting for the agent's
 * answer to "Why?": only such a ticket may be allowed with others.
 */
fun settled(item: PendingItem): Boolean {
    val t = item.ticket ?: return true
    return t.state != STATE_STALE && t.stale == null && t.state != STATE_NEEDS_INFO
}

/** Ticket states the app acts on. */
const val STATE_STALE = "stale"
const val STATE_NEEDS_INFO = "needs_info"

private fun kindOf(item: PendingItem): String = "${item.service}/${item.action}/${item.op}"

/** The groups of two or more tickets that can be answered together, in inbox order. */
fun batchGroups(pending: List<PendingItem>): List<TicketGroup> =
    waitingInbox(pending)
        .groupBy { it.connectionId to kindOf(it) }
        .filterValues { it.size >= 2 }
        .map { (key, items) -> TicketGroup(key.first, items.first().connectionLabel, key.second, items) }

/** When to remind: at 50 % and 90 % of the ticket's window. */
fun reminderTimes(ticket: TicketView): List<Long> {
    val span = (ticket.deadline - ticket.deferredAt).coerceAtLeast(0)
    return listOf(ticket.deferredAt + span / 2, ticket.deferredAt + span * 9 / 10)
}

/** "Due in 3 h", "Due in 45 min", "Due in 2 d"; "Overdue" once the deadline has passed. */
fun dueIn(deadline: Long, nowSeconds: Long): String {
    val left = deadline - nowSeconds
    return when {
        left <= 0 -> "Overdue"
        left < 3_600 -> "Due in ${((left + 59) / 60).coerceAtLeast(1)} min"
        left < 86_400 -> "Due in ${left / 3_600} h"
        else -> "Due in ${left / 86_400} d"
    }
}

/** A ticket's deadline is under an hour away (or past): its arrival and reminders are not quiet. */
fun TicketView.pressing(nowSeconds: Long): Boolean = deadline - nowSeconds < 3_600
