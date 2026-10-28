package dev.reins.android.ui.activity

import dev.reins.android.AppContainer
import dev.reins.android.design.Glyph
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Authenticator
import dev.reins.android.ui.common.userMessage
import dev.reins.core.ActivityHold
import dev.reins.core.HoldState
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.async
import kotlinx.coroutines.launch

// The email send hold (Decisions v2, 08 R10): an approved Gmail send waits out its hold on its Activity row, "Sending in
// 23 s" with Undo; one that went too late asks "Still send?" with Send and Discard.

/** A button on a held send's row: its word, its Material Symbol, its test tag. */
enum class HoldButton(val label: String, val glyph: Glyph, val tag: String) {
    /** While it waits: it is not sent (`undoSend`). */
    Undo("Undo", Glyph.Undo, "holdUndo"),

    /** "Still send?": it goes now (`sendLate`). */
    Send("Send", Glyph.Send, "holdSend"),

    /** "Still send?": it is dropped (`undoSend`). */
    Discard("Discard", Glyph.Close, "holdDiscard"),
}

/** What a held send's row shows: a few words and its buttons. */
data class HoldLine(val text: String, val buttons: List<HoldButton>)

/** Seconds until [hold] goes, never below zero. */
fun holdSecondsLeft(hold: ActivityHold, nowSeconds: Long): Long = (hold.releaseAt - nowSeconds).coerceAtLeast(0)

/** "Sending in 23 s" while there is time left, then "Sending". */
fun holdCountdown(hold: ActivityHold, nowSeconds: Long): String {
    val left = holdSecondsLeft(hold, nowSeconds)
    return if (left > 0) "Sending in $left s" else SENDING
}

/**
 * The words and buttons of a held send. Waiting: the countdown and Undo, until it runs out (then it is being released
 * and Undo would race it). Sending: no buttons. Late: "Still send?" with Send and Discard.
 */
fun holdLine(hold: ActivityHold, nowSeconds: Long): HoldLine = when (hold.state) {
    HoldState.WAITING ->
        if (holdSecondsLeft(hold, nowSeconds) > 0) HoldLine(holdCountdown(hold, nowSeconds), listOf(HoldButton.Undo)) else HoldLine(SENDING, emptyList())
    HoldState.SENDING -> HoldLine(SENDING, emptyList())
    HoldState.LATE -> HoldLine(STILL_SEND, listOf(HoldButton.Send, HoldButton.Discard))
}

/** Whether [hold]'s countdown ran out on this phone, so the app releases it now. */
fun holdDue(hold: ActivityHold, nowSeconds: Long): Boolean = hold.state == HoldState.WAITING && nowSeconds >= hold.releaseAt

/** The send-hold outcomes' own words: approved but waiting ("held"), undone, or not sent (late, Lockdown, the policy). */
fun holdOutcomeLabel(outcome: String, live: Boolean = true): String? = when (outcome) {
    // Once its hold ended, how it went is a newer entry ("sent", "undone", ...): this one only says it was held.
    "held" -> if (live) SENDING else "Held"
    "undone" -> "Undone"
    "not_sent" -> "Not sent"
    else -> null
}

/**
 * A held send's button, answered (08 R10). Send is an approval of the email, so the owner is checked first; Undo and
 * Discard only stop it. The call runs on the app's scope (leaving the screen never cuts a send in half); Activity is
 * read again after it. Returns what to say when it did not go through, or null.
 */
suspend fun answerHold(container: AppContainer, authenticator: Authenticator, hold: ActivityHold, button: HoldButton): String? {
    if (button == HoldButton.Send) {
        when (authenticator.authenticate("Send email", STILL_SEND)) {
            AuthResult.Success -> Unit
            AuthResult.Cancelled -> return null
            AuthResult.Unavailable -> {
                container.feedback.play(Event.Error)
                return "Set a screen lock or fingerprint on this phone to approve."
            }
        }
    }
    container.feedback.play(if (button == HoldButton.Send) Event.Approved else Event.Undo)
    val work = container.appScope.launchCatching {
        if (button == HoldButton.Send) container.core.sendLate(hold.id) else container.core.undoSend(hold.id)
    }
    val failure = work.await()
    container.refreshPending()
    if (failure == null) return null
    container.feedback.play(Event.Error)
    return (if (button == HoldButton.Send) "Not sent" else "Not undone") + ": ${failure.userMessage()}"
}

/** Sends the held emails that are due, on the app's scope, then reads Activity again (a late one shows "Still send?"). */
fun releaseDueHolds(container: AppContainer) {
    container.appScope.launch {
        try {
            container.core.releaseHolds()
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            // Offline or not yet due on the server: the row tries again, and the push and every sync release it too.
        }
        container.refreshPending()
    }
}

/** Runs [block] on this scope; its failure (null: none) once it ends, whoever waits for it. */
private fun kotlinx.coroutines.CoroutineScope.launchCatching(block: suspend () -> Unit): kotlinx.coroutines.Deferred<Exception?> =
    async {
        try {
            block()
            null
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            e
        }
    }

/** The send hold's choices in Settings, in seconds (0: off). */
val SEND_HOLD_CHOICES: List<UInt> = listOf(0u, 10u, 30u, 60u, 120u)

/** "Off", "10 s", "30 s". */
fun sendHoldLabel(secs: UInt): String = if (secs == 0u) "Off" else "$secs s"

const val SEND_HOLD_TITLE = "Send hold"

/** Behind the setting's (?). */
const val SEND_HOLD_HELP =
    "Approved emails wait this long before they go, with Undo in Activity. If this phone wakes more than 15 minutes late, Reins asks before sending."

const val SENDING = "Sending"
const val STILL_SEND = "Still send?"
