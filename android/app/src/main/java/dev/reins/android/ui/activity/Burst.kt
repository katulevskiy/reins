package dev.reins.android.ui.activity

import dev.reins.android.AppContainer
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Authenticator
import dev.reins.android.ui.common.untrusted
import dev.reins.core.PendingItem
import dev.reins.core.PendingKind

/**
 * Several requests from one AI at once (an agent run, or one `reins_batch`): [quick] can be approved together without opening each, [all] is
 * everything of it that waits. Only a burst with at least two routine requests is offered. Bursts of several AIs are one
 * ([ais] > 1, [connectionId] empty): their routine requests together, and "Deny" takes back only those.
 */
data class Burst(
    val connectionId: String,
    val label: String,
    val quick: List<String>,
    val all: List<String>,
    val ais: Int = 1,
    /** The batch's key ([burstKey]) for a batch, else the AI's connection. */
    val key: String = connectionId,
) {
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

/** What a request is grouped by: its batch (`reins_batch`), else the AI that asked. */
fun burstKey(item: PendingItem): String = item.batch?.let { "batch:${it.id}" } ?: item.connectionId

/**
 * The calls of a batch are their own burst, in the order the AI made them, apart from the rest of that AI's requests;
 * each is still its own request, approved, denied and recorded on its own.
 */
fun bursts(pending: List<PendingItem>): List<Burst> =
    pending
        .filter { it.kind == PendingKind.REQUEST }
        .groupBy(::burstKey)
        .mapNotNull { (_, group) ->
            val items = group.sortedBy { it.batch?.index ?: 0u }
            val quick = items.filter { it.quick }.map { it.id }
            val first = items.first()
            if (quick.size < 2) null else Burst(first.connectionId, first.connectionLabel, quick, items.map { it.id }, key = burstKey(first))
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
 * untouched; anything asked every time keeps waiting) or "Deny all". The cards go at once and the answers go out in the
 * background; any that do not come back with a short message. Returns what went wrong before anything was sent, or null.
 */
suspend fun answerBurst(container: AppContainer, authenticator: Authenticator, burst: Burst, approve: Boolean): String? {
    val ids = if (approve) burst.quick else burst.all
    if (approve) {
        when (authenticator.authenticate(if (ids.size == 1) "Approve request" else "Approve ${ids.size} requests", burst.label)) {
            AuthResult.Success -> Unit
            AuthResult.Cancelled -> return null
            AuthResult.Unavailable -> {
                container.feedback.play(Event.Error)
                return "Set a screen lock or fingerprint on this phone to approve."
            }
        }
    }
    container.feedback.play(if (approve) Event.Approved else Event.Denied)
    val failed = if (approve) "Not approved" else "Not denied"
    for (id in ids) {
        container.answerInBackground(id, failed) { if (approve) container.core.approveQuick(id) else container.core.deny(id) }
    }
    return null
}
