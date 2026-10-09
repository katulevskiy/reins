package dev.reins.android.ui.activity

import dev.reins.android.AppContainer
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Authenticator
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.common.userMessage
import dev.reins.core.PendingItem
import dev.reins.core.PendingKind
import kotlin.coroutines.cancellation.CancellationException

/**
 * Several requests from one AI at once (an agent run): [quick] can be approved together without opening each, [all] is
 * everything of it that waits. Only a burst with at least two routine requests is offered. Bursts of several AIs are one
 * ([ais] > 1, [connectionId] empty): their routine requests together, and "Deny" takes back only those.
 */
data class Burst(val connectionId: String, val label: String, val quick: List<String>, val all: List<String>, val ais: Int = 1) {
    /** Says who asks: one AI's name, or for several their names ("Claude and Codex", "Claude, Codex and 2 more"). */
    val title: String
        get() = if (ais > 1) "${quick.size} routine · ${untrusted(label)}" else "${untrusted(label)} · ${all.size} waiting"

    /** What stays in the list for a closer look, or null. */
    val heldNote: String?
        get() = when (val held = all.size - quick.size) {
            0 -> null
            1 -> "1 needs a closer look"
            else -> "$held need a closer look"
        }
}

fun bursts(pending: List<PendingItem>): List<Burst> =
    pending
        .filter { it.kind == PendingKind.REQUEST }
        .groupBy { it.connectionId }
        .mapNotNull { (connection, items) ->
            val quick = items.filter { it.quick }.map { it.id }
            if (quick.size < 2) null else Burst(connection, items.first().connectionLabel, quick, items.map { it.id })
        }

/** The one bar Activity shows above the requests: one AI's burst, or the bursts of several folded together. */
fun burstBar(pending: List<PendingItem>): Burst? {
    val all = bursts(pending)
    if (all.size <= 1) return all.firstOrNull()
    val quick = all.flatMap { it.quick }
    return Burst(connectionId = "", label = names(all.map { it.label }), quick = quick, all = quick, ais = all.size)
}

/** "Claude", "Claude and Codex", "Claude, Codex and 2 more". */
private fun names(labels: List<String>): String = when (labels.size) {
    1 -> labels[0]
    2 -> "${labels[0]} and ${labels[1]}"
    else -> "${labels[0]}, ${labels[1]} and ${labels.size - 2} more"
}

/**
 * "Approve all" (one screen lock or biometric check for the lot, then each routine request as its sheet would approve it
 * untouched; anything asked every time keeps waiting) or "Deny all" (every request of that AI). Returns what went
 * wrong, or null.
 */
suspend fun answerBurst(container: AppContainer, authenticator: Authenticator, burst: Burst, approve: Boolean): String? {
    val ids = if (approve) burst.quick else burst.all
    if (approve) {
        when (authenticator.authenticate("Approve ${ids.size} requests", burst.label)) {
            AuthResult.Success -> Unit
            AuthResult.Cancelled -> return null
            AuthResult.Unavailable -> {
                container.feedback.play(Event.Error)
                return "Set a screen lock or fingerprint on this phone to approve."
            }
        }
    }
    container.feedback.play(if (approve) Event.Approved else Event.Denied)
    var failed = 0
    var reason: String? = null
    for (id in ids) {
        try {
            if (approve) container.core.approveQuick(id) else container.core.deny(id)
            container.state.removePending(id)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            failed++
            reason = reason ?: e.userMessage()
        }
    }
    container.refreshPending()
    if (failed == 0) return null
    container.feedback.play(Event.Error)
    return "$failed of ${ids.size} could not be ${if (approve) "approved" else "denied"}: $reason"
}
