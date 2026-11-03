package dev.reins.android.ui

import dev.reins.android.TestData
import dev.reins.android.ui.activity.*
import dev.reins.android.ui.common.*
import dev.reins.core.ActivityOutcome
import dev.reins.core.ActivityDecider
import dev.reins.core.ExecutionState
import org.junit.Assert.*
import org.junit.Test

class ActivityOutcomeSafetyTest {
    @Test fun uncertainAndWithheldOutcomesOverrideEveryDeciderAndGrantSuccess() {
        val states = mapOf("started" to Outcome.Pending, "unknown" to Outcome.Unknown, "succeeded_withheld" to Outcome.Withheld)
        for ((status, expected) in states) for (decider in listOf("", "autopilot", "overnight", "bypass", "lockdown", "account_permission", "temporary_permission", "future_actor")) {
            val entry = TestData.entry(1).copy(outcome = status, decidedBy = decider, grantId = "grant")
            assertEquals(expected, outcomeOf(entry))
            assertEquals(ActivityOutcome.FAILED, outcomeKind(status))
            assertEquals(Tense.Refused, entryTense(status))
            assertFalse(outcomeLine(entry).contains("Allowed"))
            if (status != "started") assertTrue(outcomeLine(entry).contains("do not repeat"))
        }
    }
    @Test fun aFoldedExecutionTellsOnItsDecisionRowOnlyWhenSomethingIsWrong() {
        val decided = TestData.entry(1, "write", "sent", decidedBy = "bypass", service = "desktop", op = "ssh")
        val fine = decided.copy(execution = TestData.execution(ExecutionState.DONE, listOf(0L), startedAt = 1))
        assertEquals(Outcome.Bypass, outcomeOf(fine))
        assertEquals("Allowed during a bypass", outcomeLine(fine))
        assertEquals(ActivityOutcome.APPROVED, outcomeKind(fine))
        assertEquals(Tense.Done, entryTense(fine))
        val unknown = decided.copy(execution = TestData.execution(ExecutionState.UNKNOWN, listOf(0L)))
        assertEquals(Outcome.Unknown, outcomeOf(unknown))
        assertEquals("Outcome unknown · do not repeat", outcomeLine(unknown))
        assertEquals(ActivityOutcome.FAILED, outcomeKind(unknown))
        assertEquals("never says it ran", Tense.Refused, entryTense(unknown))
        val withheld = decided.copy(execution = TestData.execution(ExecutionState.WITHHELD))
        assertEquals(Outcome.Withheld, outcomeOf(withheld))
        assertTrue(outcomeLine(withheld).contains("do not repeat"))
        // A decided row says how it was decided while its execution runs; a record with nothing else is in progress.
        assertEquals(Outcome.Bypass, outcomeOf(decided.copy(execution = TestData.execution(ExecutionState.PENDING))))
        val running = TestData.entry(2, "execute", "started", execution = TestData.execution(ExecutionState.PENDING, listOf(2L)))
        assertEquals(Outcome.Pending, outcomeOf(running))
        assertEquals("In progress", outcomeLine(running))
    }
    @Test fun explicitPermissionsHaveTheirOwnAttributionWithoutLearnedConfidence() {
        for ((actor, name) in listOf("account_permission" to "account permission", "temporary_permission" to "temporary permission")) {
            val entry = TestData.entry(1).copy(outcome = "sent", decidedBy = actor, autopilot = TestData.note())
            assertEquals(Outcome.Permission, outcomeOf(entry))
            assertEquals("Allowed by $name", outcomeLine(entry))
            assertEquals("Denied · $name", outcomeLine(entry.copy(outcome = "denied")))
            assertEquals(Outcome.Denied, outcomeOf(entry.copy(outcome = "denied")))
            assertEquals(ActivityDecider.AUTOPILOT, deciderKind(actor))
            assertTrue(HistoryFilter.Autopilot.matches(entry))
            assertFalse(outcomeLine(entry).contains("Autopilot"))
            assertFalse(outcomeLine(entry).contains("%"))
        }
        assertEquals("Automatic", deciderName(ActivityDecider.AUTOPILOT))
        assertEquals("Automatic", HistoryFilter.Autopilot.label)
    }
    @Test fun unknownActorsDoNotClaimOwnerApprovalOrLearnedAutopilot() {
        val entry = TestData.entry(1).copy(outcome = "sent", decidedBy = "future_actor", autopilot = TestData.note())
        assertEquals("Allowed", outcomeLine(entry))
        assertEquals("Denied", outcomeLine(entry.copy(outcome = "denied")))
        assertEquals("Access given", outcomeLine(entry.copy(outcome = "granted")))
        assertEquals(Outcome.Approved, outcomeOf(entry))
        assertEquals(ActivityDecider.AUTOPILOT, deciderKind(entry.decidedBy))
        assertEquals("Allowed by you", outcomeLine(entry.copy(decidedBy = "")))
        assertEquals("Allowed by Autopilot · 98%", outcomeLine(entry.copy(decidedBy = "autopilot")))
    }
    @Test fun onlyKnownCompletedOutcomesUsePastTenseAndApprovedFilter() {
        for (status in listOf("sent", "released", "granted")) {
            assertEquals(Tense.Done, entryTense(status))
            assertEquals(ActivityOutcome.APPROVED, outcomeKind(status))
        }
        assertEquals("Needs attention", outcomeName(ActivityOutcome.FAILED))
    }
}
