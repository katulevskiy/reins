package dev.reins.android.ui

import dev.reins.android.TestData
import dev.reins.android.ui.activity.*
import dev.reins.android.ui.common.*
import dev.reins.core.ActivityOutcome
import org.junit.Assert.*
import org.junit.Test

class ActivityOutcomeSafetyTest {
    @Test fun uncertainAndWithheldOutcomesOverrideEveryDeciderAndGrantSuccess() {
        val states = mapOf("started" to Outcome.Started, "unknown" to Outcome.Unknown, "succeeded_withheld" to Outcome.Withheld)
        for ((status, expected) in states) for (decider in listOf("", "autopilot", "overnight", "bypass", "lockdown")) {
            val entry = TestData.entry(1).copy(outcome = status, decidedBy = decider, grantId = "grant")
            assertEquals(expected, outcomeOf(entry))
            assertEquals(ActivityOutcome.FAILED, outcomeKind(status))
            assertEquals(Tense.Refused, entryTense(status))
            assertFalse(outcomeLine(entry).contains("Allowed"))
            if (status != "started") assertTrue(outcomeLine(entry).contains("do not repeat"))
        }
    }
    @Test fun onlyKnownCompletedOutcomesUsePastTenseAndApprovedFilter() {
        for (status in listOf("sent", "released", "granted")) {
            assertEquals(Tense.Done, entryTense(status))
            assertEquals(ActivityOutcome.APPROVED, outcomeKind(status))
        }
        assertEquals("Needs attention", outcomeName(ActivityOutcome.FAILED))
    }
}
