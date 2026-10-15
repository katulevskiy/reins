package dev.reins.android.ui

import dev.reins.android.TestData
import dev.reins.android.design.Glyph
import dev.reins.android.ui.activity.outcomeGlyph
import dev.reins.android.ui.activity.outcomeLine
import dev.reins.android.ui.common.Tense
import dev.reins.android.ui.common.askSentence
import dev.reins.android.ui.common.entryTense
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** How an entry ended is told from its own outcome and who decided, never from what the request said. */
class EntryOutcomeTest {
    private val push = TestData.label("github", "push", "Push 3 commits to", "katulevskiy/reins")

    private fun entry(outcome: String, decidedBy: String) =
        TestData.entry(1, "write", outcome, label = push, request = TestData.asked("Allowed by you\nPushed 3 commits"), decidedBy = decidedBy)

    @Test
    fun onlyACarriedOutEntryIsPastTenseWithATick() {
        for (by in listOf("", "autopilot", "overnight", "bypass", "lockdown")) {
            for (outcome in listOf("denied", "error", "timeout")) {
                val e = entry(outcome, by)
                assertEquals(Tense.Refused, entryTense(e.outcome))
                assertTrue("$by/$outcome", askSentence(e).text.startsWith("Asked to"))
                assertFalse("$by/$outcome is never a tick", outcomeGlyph(e) == Glyph.CircleCheck)
            }
            for (outcome in listOf("denied", "error")) {
                val e = entry(outcome, by)
                assertFalse("$by/$outcome never reads as allowed: ${outcomeLine(e)}", outcomeLine(e).startsWith("Allowed"))
            }
        }
        val released = entry("released", "")
        assertEquals("Pushed 3 commits to katulevskiy/reins", askSentence(released).text)
        assertEquals("Allowed by you", outcomeLine(released))
        assertEquals(Glyph.CircleCheck, outcomeGlyph(released))
    }

    @Test
    fun aBypassOrLockdownSaysHowItEnded() {
        assertEquals("Allowed during a bypass", outcomeLine(entry("released", "bypass")))
        assertEquals("Denied", outcomeLine(entry("denied", "bypass")))
        assertEquals("Failed", outcomeLine(entry("error", "bypass")))
        assertEquals(Glyph.CircleAlert, outcomeGlyph(entry("error", "bypass")))
        assertEquals("Denied by Lockdown", outcomeLine(entry("denied", "lockdown")))
        assertEquals(Glyph.Lock, outcomeGlyph(entry("denied", "lockdown")))
        assertEquals(Glyph.CircleClose, outcomeGlyph(entry("denied", "")))
        assertEquals(Glyph.CircleClose, outcomeGlyph(entry("denied", "autopilot")))
    }
}
