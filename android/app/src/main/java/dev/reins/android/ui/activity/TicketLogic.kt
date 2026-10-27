package dev.reins.android.ui.activity

import dev.reins.core.PendingItem
import dev.reins.core.TicketView

// Waiting for you (Decisions v2, 03 §5.2): requests that became tickets. The agent carries on and collects the outcome
// later, so they wait in their own section above Today, soonest deadline first, by agent and task; never on the stack.
// What Overnight parked (01 §11) waits here too: a ticket once parked, and here even before its ticket shows.

/** Parked by Overnight for the morning, and the morning has not come ([nowSeconds]). */
fun PendingItem.parked(nowSeconds: Long): Boolean = (parkedUntil ?: 0) > nowSeconds

/**
 * Waits under Waiting for you: a ticket, or something Overnight parked. A task proposal never does: it has a card of its
 * own (`TaskCards`).
 */
fun PendingItem.inInbox(): Boolean = taskProposal == null && (ticket != null || parkedUntil != null)

/** Which comes first in the inbox: the deadline, else (parked, no ticket yet) the morning. */
private fun dueAt(item: PendingItem): Long = item.ticket?.deadline ?: item.parkedUntil ?: Long.MAX_VALUE

/** What waits for the user under Waiting for you, soonest deadline first. */
fun waitingInbox(pending: List<PendingItem>): List<PendingItem> =
    pending.filter { it.inInbox() }.sortedWith(compareBy<PendingItem> { dueAt(it) }.thenBy { it.createdAt })

/**
 * One agent's waiting requests under one task ([taskId] null: none), soonest deadline first. "Approve all" covers only
 * the [batchable] ones (soft and below L3, as the core marks them); the rest are answered one by one.
 */
data class AgentTickets(
    val connectionId: String,
    val label: String,
    val items: List<PendingItem>,
    val taskId: String? = null,
    val taskTitle: String? = null,
) {
    val batchable: List<PendingItem> get() = items.filter { it.batchable }

    /** For tags: the agent, and its task when there is one. */
    val key: String get() = if (taskId == null) connectionId else "$connectionId:$taskId"
}

/** The inbox by agent and task, the group with the soonest deadline first. */
fun inboxByAgent(pending: List<PendingItem>): List<AgentTickets> =
    waitingInbox(pending)
        .groupBy { it.connectionId to it.taskId }
        .map { (key, items) -> AgentTickets(key.first, items.first().connectionLabel, items, key.second, items.firstNotNullOfOrNull { it.taskTitle }) }

/**
 * One agent's tickets of one kind (the same service, action and operation), answered together. Allow takes only the
 * routine ones ([PendingItem.quick]) that are settled ([settled]): it never crosses what has to be looked at one by
 * one. Deny takes them all.
 */
data class TicketGroup(
    val connectionId: String,
    val label: String,
    val kind: String,
    val items: List<PendingItem>,
    val taskId: String? = null,
) {
    val allowIds: List<String> get() = items.filter { it.quick && settled(it) }.map { it.id }
    val denyIds: List<String> get() = items.map { it.id }

    /** The group as the stack's burst, for [answerBurst]: one owner's check, then each request on its own. */
    val burst: Burst get() = Burst(connectionId, label, allowIds, denyIds)

    /** For tags: the agent (and its task, when there is one) and the kind. */
    val key: String get() = if (taskId == null) "$connectionId:$kind" else "$connectionId:$taskId:$kind"
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

/** The groups of two or more tickets that can be answered together (one agent, one task, one kind), in inbox order. */
fun batchGroups(pending: List<PendingItem>): List<TicketGroup> =
    waitingInbox(pending)
        .groupBy { Triple(it.connectionId, it.taskId, kindOf(it)) }
        .filterValues { it.size >= 2 }
        .map { (key, items) -> TicketGroup(key.first, items.first().connectionLabel, key.third, items, key.second) }

/**
 * When to remind: at 50 % and 90 % of the ticket's window. A parked one's window starts at the morning
 * ([parkedUntil]): nothing reminds before it (01 §11).
 */
fun reminderTimes(ticket: TicketView, parkedUntil: Long? = null): List<Long> {
    val start = maxOf(ticket.deferredAt, parkedUntil ?: Long.MIN_VALUE)
    val span = (ticket.deadline - start).coerceAtLeast(0)
    return listOf(start + span / 2, start + span * 9 / 10)
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
