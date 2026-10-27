package dev.reins.android.ui

import dev.reins.android.ui.nav.SheetTarget
import dev.reins.core.PendingItem

/**
 * Which asks reach the owner. The core's list (`pending`) holds only what waits for the owner: a bypass, Lockdown,
 * Overnight, Autopilot in Auto and standing grants decide first, and what they decide never enters it. On top of that
 * list, an ask a mode or Autopilot decided meanwhile closes itself, and never pops up again, even from a list read a
 * moment before the decision (`AppState.markAutoDecided` keeps it out).
 */
object AskGate {
    /**
     * The item to pop up by itself: the first one that waits for the owner and has not popped up before. A ticket never
     * pops up: the agent is not waiting on it, and it waits under "Waiting for you".
     */
    fun next(waiting: List<PendingItem>, presented: Set<String>): PendingItem? = waiting.firstOrNull { it.id !in presented && it.ticket == null }

    /** Whether the sheet open on [open] is the ask a mode or Autopilot just decided ([decidedId]): it closes. */
    fun closes(open: SheetTarget?, decidedId: String): Boolean = open != null && open.id == decidedId
}
