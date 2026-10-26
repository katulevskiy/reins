package dev.reins.android.platform.widgets

import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.design.Field
import dev.reins.android.ui.activity.Outcome
import dev.reins.android.ui.activity.cardWho
import dev.reins.android.ui.activity.compactTime
import dev.reins.android.ui.activity.outcomeOf
import dev.reins.android.ui.activity.rowAction
import dev.reins.android.ui.common.untrustedName
import dev.reins.android.ui.activity.field
import dev.reins.android.ui.activity.pendingTone
import dev.reins.android.ui.common.askSentence
import dev.reins.core.ActivityEntry
import dev.reins.core.AutopilotMode
import dev.reins.core.AutopilotSettings
import dev.reins.core.PendingItem
import dev.reins.core.PendingKind

/**
 * What the Waiting widget shows (And-Outside 04): the top card of the stack, its field and all, or with nothing waiting,
 * paper and the pond. Never a way to approve: the widget opens the request.
 */
data class WaitingWidgetModel(val waiting: Int, val top: TopCard?, val rows: List<WaitingRow> = emptyList()) {
    /** A line of the waiting list (the wide widget): the icon, the ask, and who asks (capped, set apart). */
    data class WaitingRow(val id: String, val kind: PendingKind, val ask: String, val who: String, val provider: String, val action: String)

    /** The top card: its id and kind (to open it), the label, the ask and its italic target, who asks, the field. */
    data class TopCard(
        val id: String,
        val kind: PendingKind,
        val label: String,
        val ask: String,
        val target: String?,
        /** The machine part (a command, a branch, an address), in mono under the ask. */
        val details: String?,
        val who: String,
        val field: Field,
        /** When the AI stops waiting (seconds), for the timer; null without one. */
        val waitUntil: Long?,
        /** What it touches and what it does: the icon. */
        val provider: String = "",
        val action: String = "",
    )
}

fun waitingWidgetModel(pending: List<PendingItem>): WaitingWidgetModel {
    val first = pending.firstOrNull() ?: return WaitingWidgetModel(0, null)
    // The one formatter's words (ASK-TEXT.md), as on the card.
    val ask = askSentence(first)
    return WaitingWidgetModel(
        pending.size,
        WaitingWidgetModel.TopCard(
            id = first.id,
            kind = first.kind,
            label = ask.kind,
            ask = ask.text,
            target = ask.target,
            details = ask.details,
            who = cardWho(first),
            field = pendingTone(first, null).field ?: Field.Push,
            waitUntil = first.waitUntil.takeIf { first.kind != PendingKind.BLOB },
            provider = ask.provider,
            action = ask.action,
        ),
        pending.take(WIDGET_ROWS).map { item ->
            val words = askSentence(item)
            WaitingWidgetModel.WaitingRow(item.id, item.kind, words.text, untrustedName(item.connectionLabel, WHO_MAX), words.provider, words.action)
        },
    )
}

/** Rows a list widget shows. */
const val WIDGET_ROWS = 3

/** The longest asker's name on a widget row. */
private const val WHO_MAX = 32

/** "3 waiting". */
fun waitingCount(model: WaitingWidgetModel): String = "${model.waiting} waiting"

/**
 * What the Latest widget shows (And-Outside 05): the last few decided entries, who did what, when, and how it ended
 * (allowed, refused, or neither yet).
 */
data class LatestWidgetModel(val rows: List<Row>) {
    data class Row(
        val id: String,
        /** The asker's name, capped and drawn apart from Reins' words. */
        val who: String,
        /** Reins' words after it: "read 3 emails". */
        val action: String,
        val provider: String,
        val actionKey: String,
        val time: String,
        /** True allowed, false refused, null neither (started, unknown). */
        val allowed: Boolean?,
    )
}

fun latestWidgetModel(activity: List<ActivityEntry>, nowSeconds: Long): LatestWidgetModel =
    LatestWidgetModel(
        activity.sortedByDescending { it.at }.take(WIDGET_ROWS).map { entry ->
            val words = askSentence(entry)
            LatestWidgetModel.Row(
                id = entry.id.toString(),
                who = untrustedName(entry.connectionLabel, WHO_MAX),
                action = rowAction(words.text),
                provider = words.provider,
                actionKey = words.action,
                time = compactTime(entry.at, nowSeconds),
                allowed = when (outcomeOf(entry)) {
                    Outcome.Approved, Outcome.Auto, Outcome.Overnight, Outcome.Grant, Outcome.Permission, Outcome.Bypass, Outcome.Withheld -> true
                    Outcome.Denied, Outcome.AutoDenied, Outcome.OvernightDenied, Outcome.Lockdown, Outcome.Failed -> false
                    Outcome.Started, Outcome.Unknown -> null
                },
            )
        },
    )


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
