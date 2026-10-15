package dev.reins.android.platform.widgets

import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.design.Field
import dev.reins.android.ui.activity.cardAsk
import dev.reins.android.ui.activity.cardLabel
import dev.reins.android.ui.activity.cardWho
import dev.reins.android.ui.activity.field
import dev.reins.android.ui.activity.pendingTone
import dev.reins.core.ActivityEntry
import dev.reins.core.AutopilotMode
import dev.reins.core.AutopilotSettings
import dev.reins.core.PendingItem
import dev.reins.core.PendingKind

/**
 * What the Waiting widget shows (And-Outside 04): the top card of the stack, its field and all, or with nothing waiting,
 * paper and the pond. Never a way to approve: the widget opens the request.
 */
data class WaitingWidgetModel(val waiting: Int, val top: TopCard?) {
    /** The top card: its id and kind (to open it), the label, the ask and its italic target, who asks, the field. */
    data class TopCard(
        val id: String,
        val kind: PendingKind,
        val label: String,
        val ask: String,
        val target: String?,
        val who: String,
        val field: Field,
        /** When the AI stops waiting (seconds), for the timer; null without one. */
        val waitUntil: Long?,
    )
}

fun waitingWidgetModel(pending: List<PendingItem>): WaitingWidgetModel {
    val first = pending.firstOrNull() ?: return WaitingWidgetModel(0, null)
    val ask = cardAsk(first, null)
    return WaitingWidgetModel(
        pending.size,
        WaitingWidgetModel.TopCard(
            id = first.id,
            kind = first.kind,
            label = cardLabel(first),
            ask = ask.text,
            target = ask.target,
            who = cardWho(first),
            field = pendingTone(first, null).field ?: Field.Push,
            waitUntil = first.waitUntil.takeIf { first.kind != PendingKind.BLOB },
        ),
    )
}

/** "WAITING · 2". */
fun waitingLabel(model: WaitingWidgetModel): String = "Waiting · ${model.waiting}"

/** What the Autopilot widget shows: the mode, how much it decided today, and whether Lock down is offered. */
data class AutopilotWidgetModel(val mode: AutopilotMode, val decidedToday: Int) {
    val name: String get() = AutopilotText.name(mode)

    /** Lock down is offered unless it already is: it only takes power away, so it needs no check. */
    val offersLockDown: Boolean get() = mode != AutopilotMode.LOCKDOWN

    val line: String get() = when (decidedToday) {
        0 -> "Nothing decided today"
        else -> "$decidedToday decided today"
    }
}

fun autopilotWidgetModel(settings: AutopilotSettings?, activity: List<ActivityEntry>, nowSeconds: Long): AutopilotWidgetModel =
    AutopilotWidgetModel(settings?.mode ?: AutopilotMode.MANUAL, dev.reins.android.ui.trust.decidedToday(activity, nowSeconds))
