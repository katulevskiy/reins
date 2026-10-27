package dev.reins.android.ui

import dev.reins.android.TestData
import dev.reins.android.ui.trust.RunningListFold
import dev.reins.android.ui.trust.runningEntries
import dev.reins.android.ui.trust.decidedLine
import dev.reins.android.ui.trust.decidedToday
import dev.reins.android.ui.trust.runningListTitle
import dev.reins.android.ui.trust.runningMood
import dev.reins.core.AutopilotMode
import dev.reins.core.AutopilotSettings
import dev.reins.core.ModelState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** The list under a running bypass, Overnight or lockdown on Trust: what it holds and how it folds. */
class RunningListTest {
    private fun settings(mode: AutopilotMode, until: Long? = null, since: Long? = null) = AutopilotSettings(
        mode = mode,
        baseMode = AutopilotMode.MANUAL,
        bypassUntil = until.takeIf { mode == AutopilotMode.BYPASS },
        bypassSince = since.takeIf { mode == AutopilotMode.BYPASS },
        overnightUntil = until.takeIf { mode == AutopilotMode.OVERNIGHT },
        overnightSince = since.takeIf { mode == AutopilotMode.OVERNIGHT },
        defaultProfileId = "personal",
        wifiOnly = true,
        model = TestData.modelStatus(ModelState.INSTALLED),
        connections = emptyList(),
    )

    @Test
    fun aHundredApprovalsFoldToTheLatestThree() {
        val entries = (1L..100L).map { TestData.entry(it, at = 1_000 + (it * 37) % 100, decidedBy = "bypass") }.shuffled() +
            TestData.entry(500, at = 1_050, decidedBy = "autopilot") +
            TestData.entry(501, at = 10, decidedBy = "bypass")
        val mine = runningEntries(entries, settings(AutopilotMode.BYPASS, until = 5_000, since = 900))!!
        assertEquals("only what this bypass decided", 100, mine.size)
        assertEquals("newest first", mine.map { it.at }.sortedDescending(), mine.map { it.at })
        assertEquals(3, RunningListFold.shown(mine.size, expanded = false))
        assertEquals(100, RunningListFold.shown(mine.size, expanded = true))
        assertEquals("Show all 97", RunningListFold.showAll(mine.size))
        assertEquals(2, RunningListFold.shown(2, expanded = false))
        assertNull("nothing to open", RunningListFold.showAll(3))
        assertEquals("Show all 1", RunningListFold.showAll(4))
    }

    @Test
    fun whileABypassOrOvernightRunsTheMoodSaysWhatItDidToday() {
        // Mid-morning wherever the test runs, so twelve minutes earlier is still today.
        val midnight = java.util.Calendar.getInstance().apply {
            timeInMillis = 1_800_000_000_000
            set(java.util.Calendar.HOUR_OF_DAY, 0)
            set(java.util.Calendar.MINUTE, 0)
            set(java.util.Calendar.SECOND, 0)
            set(java.util.Calendar.MILLISECOND, 0)
        }.timeInMillis / 1000
        val now = midnight + 10 * 3_600
        val bypassed = (1L..100L).map { TestData.entry(it, at = now - 600 + it, decidedBy = "bypass") }
        val bypass = settings(AutopilotMode.BYPASS, until = now + 3_600, since = now - 700)
        val listed = runningEntries(bypassed, bypass)
        assertEquals("counted from the list", "Bypass approved 100 today.", runningMood(bypass, listed, now))
        assertEquals("what it said before", "Autopilot decided nothing yet today.", decidedLine(decidedToday(bypassed, now)))
        assertEquals("Bypass approved nothing yet today.", runningMood(bypass, emptyList(), now))
        val yesterday = TestData.entry(200, at = midnight - 60, decidedBy = "bypass")
        assertEquals("today only", "Bypass approved 1 today.", runningMood(bypass, listOf(yesterday) + listed!!.take(1), now))
        val overnight = settings(AutopilotMode.OVERNIGHT, until = now + 3_600, since = 0)
        assertEquals("${dev.reins.android.autopilot.AutopilotText.OVERNIGHT} decided 12 today.", runningMood(overnight, listed.take(12), now))
        assertNull("a lockdown keeps Autopilot's line", runningMood(settings(AutopilotMode.LOCKDOWN), listed, now))
        assertNull("nothing runs", runningMood(settings(AutopilotMode.MANUAL), runningEntries(bypassed, settings(AutopilotMode.MANUAL)), now))
    }

    @Test
    fun overnightAndLockdownListTheSameWay() {
        val entries = listOf(TestData.entry(1, decidedBy = "overnight"), TestData.entry(2, decidedBy = "lockdown"), TestData.entry(3, decidedBy = "bypass"))
        assertEquals(listOf(1L), runningEntries(entries, settings(AutopilotMode.OVERNIGHT, until = Long.MAX_VALUE, since = 0))!!.map { it.id })
        assertEquals(listOf(2L), runningEntries(entries, settings(AutopilotMode.LOCKDOWN))!!.map { it.id })
        assertNull("nothing runs", runningEntries(entries, settings(AutopilotMode.MANUAL)))
        assertEquals("During bypass", runningListTitle(settings(AutopilotMode.BYPASS, 1, 0)))
        assertEquals("Denied", runningListTitle(settings(AutopilotMode.LOCKDOWN)))
    }
}
