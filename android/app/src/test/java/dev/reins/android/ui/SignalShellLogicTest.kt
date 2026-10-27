package dev.reins.android.ui

import dev.reins.android.TestData
import dev.reins.android.design.ThemeChoice
import dev.reins.android.ui.activity.PushKey
import dev.reins.android.ui.activity.approvedPush
import dev.reins.android.ui.activity.cardAsk
import dev.reins.android.ui.activity.cardLabel
import dev.reins.android.ui.activity.dayCount
import dev.reins.android.ui.activity.isDefaultBranch
import dev.reins.android.ui.activity.pushKey
import dev.reins.android.ui.activity.quietMood
import dev.reins.android.ui.activity.reasonLine
import dev.reins.android.ui.activity.waitingMood
import dev.reins.android.ui.connections.askersLine
import dev.reins.android.ui.main.bypassRunning
import dev.reins.android.ui.trust.decidedLine
import dev.reins.core.AutopilotMode
import dev.reins.core.Verdict
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** What the Signal shell (Activity's stack and river, Trust, Connections, Appearance) says, apart from the drawing. */
class SignalShellLogicTest {
    private val push = TestData.pending("req1", "write", 8u, service = "github", op = "git_push", opTitle = "Push to a branch")

    @Test
    fun theReasonLineCountsPushesToTheSameRepoAndBranchKindYouApproved() {
        val feature = PushKey("acme/web", defaultBranch = false)
        val approved = List(11) { feature } + PushKey("acme/web", true) + PushKey("octo/app", false)
        assertEquals(
            "Like 11 pushes to feature branches of acme/web you approved",
            reasonLine(TestData.suggestion("req1"), push, 30, 2, feature, approved),
        )
        assertEquals(
            "Like 1 push to the default branch of acme/web you approved",
            reasonLine(TestData.suggestion("req1"), push, 30, 2, PushKey("acme/web", true), approved),
        )
        // No push of that repo yet: Autopilot's own count of the kind.
        assertEquals("Like 30 pushes you approved", reasonLine(TestData.suggestion("req1"), push, 30, 2, PushKey("new/repo", false), approved))
    }

    @Test
    fun theReasonLineFollowsAutopilotsVerdictAndIsQuietWhenUnsure() {
        val search = TestData.pending("req2", "search")
        assertEquals("Like 9 searches you approved", reasonLine(TestData.suggestion("req2"), search, 9, 1, null, emptyList()))
        assertEquals("Like 3 searches you denied", reasonLine(TestData.suggestion("req2", verdict = Verdict.DENY), search, 9, 3, null, emptyList()))
        assertNull(reasonLine(TestData.suggestion("req2", verdict = Verdict.ASK), search, 9, 1, null, emptyList()))
        assertNull("an always-ask kind has no reason to be like", reasonLine(TestData.suggestion("req2", floor = true), search, 9, 1, null, emptyList()))
        assertNull(reasonLine(TestData.suggestion("req2"), search, 0, 0, null, emptyList()))
        assertNull(reasonLine(null, search, 9, 1, null, emptyList()))
    }

    @Test
    fun aPushIsKeyedByItsRepoAndWhetherItGoesToTheDefaultBranch() {
        assertTrue(isDefaultBranch("main", null))
        assertTrue(isDefaultBranch("trunk", "trunk"))
        assertFalse(isDefaultBranch("feature/login", "main"))
        val view = TestData.gitPushView()
        val key = pushKey(view)
        assertEquals(view.git!!.repo, key?.repo)
        assertNull(pushKey(TestData.searchView()))
        val mine = TestData.entry(1, "write", "sent", 1u).copy(op = "git_push")
        assertTrue(approvedPush(mine))
        assertFalse("Autopilot's push is not one you approved", approvedPush(mine.copy(decidedBy = "autopilot")))
        assertFalse(approvedPush(mine.copy(outcome = "denied")))
    }

    @Test
    fun aCardAsksInTheImperativeWithItsTargetInItalic() {
        val ask = cardAsk(TestData.pending("req1", "search", 1u).copy(subtitle = "invoice"), null)
        assertEquals("Search Gmail for invoice", ask.text)
        assertEquals("invoice", ask.target)
        val labelled = cardAsk(push.copy(label = TestData.label("github", "push", "Push 8 commits to", "octo/app")), null)
        assertEquals("Push 8 commits to octo/app", labelled.text)
        assertEquals("octo/app", labelled.target)
        val code = cardAsk(push.copy(label = TestData.label("terminal", "run", "Run", "git push", code = true)), null)
        assertEquals("a command never goes into the serif", "Run a command", code.text)
        assertNull(code.target)
        assertEquals("git push", code.details)
        assertEquals("GitHub · Push", cardLabel(push.copy(label = TestData.label("github", "push", "Push", "octo/app"))))
    }

    @Test
    fun theMoodLinesCountInWords() {
        assertEquals("One" to " thing waits for you.", waitingMood(1))
        assertEquals("Two" to " things wait for you.", waitingMood(2))
        assertEquals("All quiet. Three AIs are connected.", quietMood(3))
        assertEquals("All quiet. Nothing is connected yet.", quietMood(0))
        assertEquals("Autopilot decided 9 things today.", decidedLine(9))
        assertEquals("Autopilot decided nothing yet today.", decidedLine(0))
        val ais = listOf(TestData.connection("c1", "Claude"), TestData.connection("c2", "ChatGPT"), TestData.connection("c3", "Cursor"))
        assertEquals("Three AIs and a computer can ask.", askersLine(ais + TestData.connection("c9", "Laptop", keyFingerprint = "k")))
        assertEquals("Nothing can ask yet.", askersLine(emptyList()))
    }

    @Test
    fun todayCountsWhatWasAskedAndWhatAutopilotDecided() {
        val day = listOf(TestData.entry(1), TestData.entry(2).copy(decidedBy = "autopilot"), TestData.entry(3).copy(decidedBy = "overnight"))
        assertEquals("3 asked · 2 by Autopilot", dayCount(day, autopilotOnly = false))
        assertEquals("2 by Autopilot", dayCount(day, autopilotOnly = true))
        assertEquals("1 asked", dayCount(day.take(1), autopilotOnly = false))
    }

    @Test
    fun paperOrGraphiteOverrideTheSystem() {
        assertTrue(ThemeChoice.System.dark(systemDark = true))
        assertFalse(ThemeChoice.Paper.dark(systemDark = true))
        assertTrue(ThemeChoice.Graphite.dark(systemDark = false))
    }

    @Test
    fun theTrustDotShowsWhileAnyBypassRuns() {
        assertFalse(bypassRunning(null))
        assertFalse(bypassRunning(settings()))
        assertTrue(bypassRunning(settings().copy(mode = AutopilotMode.BYPASS)))
    }

    @Test
    fun anAccountThatNeedsConsentSaysWhatHappenedNotTheButton() {
        val needsAgain = { kind: String -> dev.reins.android.ui.services.needsAgain(kind) }
        assertEquals("Token expired", needsAgain("google"))
        assertEquals("Access turned off", needsAgain("device"))
        assertEquals("Signed out", needsAgain("telegram"))
        assertEquals("Locked", needsAgain("vault"))
        assertFalse(listOf("google", "device", "telegram", "vault", "token").any { needsAgain(it) == "Allow again" })
    }

    @Test
    fun trustsFieldIsAmberForLockdownRedForBypassNightForOvernight() {
        fun trustField(s: dev.reins.core.AutopilotSettings?) = dev.reins.android.ui.autopilot.trustField(s)
        assertNull(trustField(null))
        assertNull(trustField(settings()))
        assertEquals(dev.reins.android.design.Field.Warn, trustField(settings().copy(mode = AutopilotMode.LOCKDOWN)))
        assertEquals("the iOS-Trust board's amber", androidx.compose.ui.graphics.Color(0xFFA16207), dev.reins.android.design.Field.Warn.fill())
        assertEquals(dev.reins.android.design.Field.Delete, trustField(settings().copy(mode = AutopilotMode.BYPASS, bypassUntil = 1_700_000_900)))
        assertEquals("Next: Overnight's night field", dev.reins.android.design.Field.Night, trustField(settings().copy(mode = AutopilotMode.OVERNIGHT, overnightUntil = 1_700_000_900)))
    }

    private fun settings() = dev.reins.core.AutopilotSettings(
        mode = AutopilotMode.MANUAL,
        baseMode = AutopilotMode.MANUAL,
        bypassUntil = null,
        bypassSince = null,
        defaultProfileId = "personal",
        wifiOnly = true,
        model = TestData.modelStatus(dev.reins.core.ModelState.INSTALLED),
        connections = emptyList(),
    )
}
