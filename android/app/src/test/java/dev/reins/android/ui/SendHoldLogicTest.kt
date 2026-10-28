package dev.reins.android.ui

import dev.reins.android.TestData
import dev.reins.android.ui.activity.HoldButton
import dev.reins.android.ui.activity.Outcome
import dev.reins.android.ui.activity.holdCountdown
import dev.reins.android.ui.activity.holdDue
import dev.reins.android.ui.activity.holdLine
import dev.reins.android.ui.activity.outcomeKind
import dev.reins.android.ui.activity.outcomeLine
import dev.reins.android.ui.activity.outcomeOf
import dev.reins.android.ui.activity.SEND_HOLD_CHOICES
import dev.reins.android.ui.activity.sendHoldLabel
import dev.reins.android.ui.common.Tense
import dev.reins.android.ui.common.askSentence
import dev.reins.android.ui.common.entryTense
import dev.reins.core.ActivityHold
import dev.reins.core.ActivityOutcome
import dev.reins.core.HoldState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The email send hold (08 R10) on a row: its countdown, its buttons, and how its outcomes read. */
class SendHoldLogicTest {
    private val now = 1_700_000_100L
    private val send = TestData.label("gmail", "send", "Send an email to", "friend@example.com")

    private fun hold(state: HoldState, inSeconds: Long = 23) = ActivityHold("h1", now + inSeconds, state)

    private fun entry(outcome: String, hold: ActivityHold? = null, decidedBy: String = "") =
        TestData.entry(1, "send", outcome, 1u, label = send, decidedBy = decidedBy, hold = hold)

    @Test
    fun theCountdownSaysHowLongIsLeftThenSending() {
        assertEquals("Sending in 23 s", holdCountdown(hold(HoldState.WAITING, 23), now))
        assertEquals("Sending in 1 s", holdCountdown(hold(HoldState.WAITING, 1), now))
        assertEquals("Sending in 120 s", holdCountdown(hold(HoldState.WAITING, 120), now))
        assertEquals("Sending", holdCountdown(hold(HoldState.WAITING, 0), now))
        // A clock past the release never counts below zero.
        assertEquals("Sending", holdCountdown(hold(HoldState.WAITING, -5), now))
    }

    @Test
    fun eachStateGetsItsWordsAndButtons() {
        val waiting = holdLine(hold(HoldState.WAITING), now)
        assertEquals("Sending in 23 s", waiting.text)
        assertEquals(listOf(HoldButton.Undo), waiting.buttons)
        // Run out: it is being released, and an Undo would race the send.
        val ranOut = holdLine(hold(HoldState.WAITING, 0), now)
        assertEquals("Sending", ranOut.text)
        assertTrue(ranOut.buttons.isEmpty())
        val sending = holdLine(hold(HoldState.SENDING), now)
        assertEquals("Sending", sending.text)
        assertTrue(sending.buttons.isEmpty())
        val late = holdLine(hold(HoldState.LATE, -1_000), now)
        assertEquals("Still send?", late.text)
        assertEquals(listOf(HoldButton.Send, HoldButton.Discard), late.buttons)
    }

    @Test
    fun theButtonsAreMaterialSymbolsWithAWordEach() {
        assertEquals(dev.reins.android.design.Glyph.Undo, HoldButton.Undo.glyph)
        assertEquals(dev.reins.android.design.Glyph.Send, HoldButton.Send.glyph)
        assertEquals(dev.reins.android.design.Glyph.Close, HoldButton.Discard.glyph)
        assertEquals(listOf("Undo", "Send", "Discard"), HoldButton.entries.map { it.label })
    }

    @Test
    fun onlyAWaitingHoldThatRanOutIsReleasedByTheApp() {
        assertFalse(holdDue(hold(HoldState.WAITING, 1), now))
        assertTrue(holdDue(hold(HoldState.WAITING, 0), now))
        assertTrue(holdDue(hold(HoldState.WAITING, -3), now))
        assertFalse(holdDue(hold(HoldState.SENDING, -3), now))
        assertFalse(holdDue(hold(HoldState.LATE, -3), now))
    }

    @Test
    fun theHoldOutcomesHaveTheirOwnWordsWhoeverDecided() {
        for (by in listOf("", "autopilot", "overnight", "account_permission")) {
            assertEquals(by, "Sending", outcomeLine(entry("held", hold(HoldState.WAITING), by)))
            assertEquals(by, "Undone", outcomeLine(entry("undone", decidedBy = by)))
            assertEquals(by, "Not sent", outcomeLine(entry("not_sent", decidedBy = by)))
            assertEquals(Outcome.Held, outcomeOf(entry("held", decidedBy = by)))
            assertEquals(Outcome.Undone, outcomeOf(entry("undone", decidedBy = by)))
            assertEquals(Outcome.NotSent, outcomeOf(entry("not_sent", decidedBy = by)))
        }
        // Once its hold ended, how it went is a newer entry: this one only says it was held, never "Sending" for ever.
        assertEquals("Held", outcomeLine(entry("held")))
        // The new outcomes never read as allowed.
        for (o in listOf("held", "undone", "not_sent")) assertFalse(o, outcomeLine(entry(o)).startsWith("Allowed"))
    }

    @Test
    fun aHeldSendIsApprovedButNotSentYet() {
        // Not past tense: "Asked to send", until its own "sent" entry.
        assertEquals(Tense.Refused, entryTense("held"))
        assertTrue(askSentence(entry("held", hold(HoldState.WAITING))).text.startsWith("Asked to"))
        assertEquals(Tense.Done, entryTense("sent"))
        for (o in listOf("undone", "not_sent")) assertEquals(o, Tense.Refused, entryTense(o))
    }

    @Test
    fun theFiltersGroupThemAsTheCoreDoes() {
        assertEquals(ActivityOutcome.APPROVED, outcomeKind("held"))
        assertEquals(ActivityOutcome.DENIED, outcomeKind("undone"))
        assertEquals(ActivityOutcome.FAILED, outcomeKind("not_sent"))
    }

    @Test
    fun theSettingOffersOffToTwoMinutes() {
        assertEquals(listOf("Off", "10 s", "30 s", "60 s", "120 s"), SEND_HOLD_CHOICES.map(::sendHoldLabel))
    }
}
