package dev.reins.android.ui

import dev.reins.android.TestData
import dev.reins.android.ui.approval.FieldWarning
import dev.reins.android.ui.approval.WarnTone
import dev.reins.android.ui.approval.dangerTag
import dev.reins.android.ui.approval.fieldWarnings
import dev.reins.android.ui.approval.heldByDanger
import dev.reins.android.ui.approval.historyFieldWarnings
import dev.reins.core.ApprovalView
import dev.reins.core.AskRuleView
import dev.reins.core.RiskLevel
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The field's warning lines, one per flag: each says its own words and none hides behind another. */
class FieldWarningsTest {
    private fun lines(view: ApprovalView, suggestion: dev.reins.core.SuggestionView? = null, risk: dev.reins.core.RiskView? = null) =
        fieldWarnings(view, suggestion, risk)

    private fun List<FieldWarning>.line(tag: String): FieldWarning = firstOrNull { it.tag == tag } ?: error("no $tag in ${map { it.tag }}")

    private val forceRule = AskRuleView("Rewriting git history", "git push --force*", "git push --force origin main", "git-history", true, false)
    private val envRule = AskRuleView(".env files", ".env", "app/.env", "env-files", false, false)

    @Test
    fun commandHistoryKeepsGuardEffectsAndUncertaintyOnceWithoutChangingWaitingWarnings() {
        val ruled = TestData.askView(rule = envRule.copy(reported = true)).let {
            it.copy(noStanding = true, ask = it.ask!!.copy(uncertain = true))
        }
        assertTrue(lines(ruled).any { it.tag == "askUncertain" })
        val history = historyFieldWarnings(ruled, command = true)
        assertEquals(listOf("guardRule"), history.map { it.tag })
        val help = history.single().help!!
        assertTrue(help.contains("details or behavior could not be verified"))
        assertTrue(help.contains("Your computer says so"))
        assertTrue(help.contains("asked every time"))
        assertEquals(lines(ruled), historyFieldWarnings(ruled, command = false))
    }

    @Test
    fun `a guard rule says its name and what matched`() {
        val w = lines(TestData.askView(rule = envRule)).line("guardRule")
        assertEquals(".env files · app/.env", w.text)
        assertEquals(WarnTone.Amber, w.tone)
    }

    @Test
    fun `a red guard rule says it always asks`() {
        val w = lines(TestData.askView(rule = forceRule).copy(noStanding = true)).line("guardRule")
        assertEquals(WarnTone.Red, w.tone)
        assertTrue(w.text.startsWith("Always asks"))
    }

    @Test
    fun `a rule the computer reported is said on its own line`() {
        val own = AskRuleView("Your rule", "make deploy", "make deploy", null, false, true)
        assertTrue(lines(TestData.askView(rule = own)).line("ruleReported").text.contains("Your computer says so"))
        assertTrue(lines(TestData.askView(rule = envRule)).none { it.tag == "ruleReported" })
    }

    @Test
    fun `a command this phone couldn't read is said, with or without a rule`() {
        val plain = TestData.askView()
        assertEquals("Unverified request details", lines(plain.copy(ask = plain.ask!!.copy(uncertain = true))).line("askUncertain").text)
        val ruled = TestData.askView(rule = envRule)
        val both = lines(ruled.copy(ask = ruled.ask!!.copy(uncertain = true)))
        assertEquals(listOf("guardRule", "askUncertain"), both.map { it.tag }.filter { it == "guardRule" || it == "askUncertain" })
    }

    @Test
    fun `a safety net is said, display only, and never loosens`() {
        val ruled = TestData.askView(rule = forceRule).copy(noStanding = true)
        val netted = ruled.copy(ask = ruled.ask!!.copy(net = "Keeps 1 deleted path in the Reins trash for 7 days"))
        val w = lines(netted).line("netted")
        assertEquals("Netted", w.text)
        assertEquals(WarnTone.Plain, w.tone)
        assertTrue(w.help!!.contains("Reins trash"))
        assertEquals(heldByDanger(ruled), heldByDanger(netted))
        assertEquals(dangerTag(ruled), dangerTag(netted))
        assertTrue(lines(ruled).none { it.tag == "netted" })
    }

    @Test
    fun `a force push rewrites history`() {
        val w = lines(TestData.gitPushView(TestData.gitPush(TestData.gitRef(force = true)), noStanding = true)).line("forceWarning:0")
        assertEquals("Always asks · rewrites history", w.text)
        assertEquals(WarnTone.Red, w.tone)
    }

    @Test
    fun `a push whose history wasn't checked`() {
        val w = lines(TestData.gitPushView(TestData.gitPush(TestData.gitRef(force = true, forceUnknown = true)), noStanding = true)).line("forceUnknown:0")
        assertEquals("Always asks · history not checked", w.text)
    }

    @Test
    fun `a force push with hidden characters shows both`() {
        val ref = TestData.gitRef(force = true, commits = listOf(TestData.gitCommit(1).copy(hiddenChars = true)))
        val all = lines(TestData.gitPushView(TestData.gitPush(ref), noStanding = true))
        assertEquals("Always asks · rewrites history", all.line("forceWarning:0").text)
        assertEquals("Always asks · hidden characters", all.line("hiddenChars").text)
        // The first-tag accessor still answers for existing callers.
        assertEquals("forceWarning:0", dangerTag(TestData.gitPushView(TestData.gitPush(ref), noStanding = true))?.tag)
    }

    @Test
    fun `a destructive tool can't be undone`() {
        assertEquals("Always asks · can't be undone", lines(TestData.mcpCallView(destructive = true)).line("mcpDestructive").text)
    }

    @Test
    fun `a delete can't be undone`() {
        val w = lines(TestData.designDeleteView()).line("onceWarning")
        assertEquals("Always asks · can't be undone", w.text)
        assertEquals(WarnTone.Red, w.tone)
    }

    @Test
    fun `a push to the default branch says so, display only`() {
        val push = TestData.gitPush(TestData.gitRef("main")).copy(defaultBranch = "main")
        val view = TestData.gitPushView(push)
        assertEquals("To the default branch main", lines(view).line("defaultBranch").text)
        assertFalse("display only: nothing is held for it", heldByDanger(view))
        val feature = TestData.gitPush(TestData.gitRef("feature/x")).copy(defaultBranch = "main")
        assertTrue(lines(TestData.gitPushView(feature)).none { it.tag == "defaultBranch" })
    }

    @Test
    fun `an ssh server with no name`() {
        assertEquals("Server known only by its key", lines(TestData.sshView(host = null)).line("unknownHost").text)
        assertEquals("Unknown server", lines(TestData.sshView(host = null, hostKey = null)).line("unknownHost").text)
        assertTrue(lines(TestData.sshView()).none { it.tag == "unknownHost" })
    }

    @Test
    fun `why it's asked every time, when no danger says so`() {
        assertEquals("Always asks · money can't come back", lines(TestData.purchaseView()).line("alwaysAsks").text)
        assertEquals("Always asks · it prints a value", lines(TestData.envReleaseView(TestData.envRelease(print = true))).line("alwaysAsks").text)
        assertTrue(lines(TestData.gitPushView(TestData.gitPush(TestData.gitRef(force = true)), noStanding = true)).none { it.tag == "alwaysAsks" })
        assertTrue(lines(TestData.searchView()).none { it.tag == "alwaysAsks" })
    }

    @Test
    fun `the first time for this AI`() {
        assertEquals("First time for this AI", lines(TestData.searchView(), TestData.suggestion(novel = true)).line("novel").text)
        assertTrue(lines(TestData.searchView(), TestData.suggestion()).none { it.tag == "novel" })
    }

    @Test
    fun `the risk wraps to two lines and the hold has its own line`() {
        val all = lines(TestData.searchView(), risk = TestData.risk(RiskLevel.CATASTROPHIC, "deletes the default branch main", held = true))
        val risk = all.line("risk")
        assertEquals("Risk: catastrophic · deletes the default branch main", risk.text)
        assertEquals(2, risk.maxLines)
        assertEquals("Held by Overnight", all.line("riskHeld").text)
        val short = fieldWarnings(TestData.searchView(), null, TestData.risk(RiskLevel.SEVERE, "a long reason"), riskInFacts = true)
        assertEquals("Risk: severe", short.line("risk").text)
        assertTrue(lines(TestData.searchView(), risk = TestData.risk()).none { it.tag == "riskHeld" })
    }

    @Test
    fun `every red line holds the action`() {
        val hidden = TestData.gitRef(commits = listOf(TestData.gitCommit(1).copy(hiddenChars = true)))
        val red = listOf(
            TestData.designDeleteView(),
            TestData.mcpCallView(destructive = true),
            TestData.gitPushView(TestData.gitPush(TestData.gitRef(force = true))),
            TestData.gitPushView(TestData.gitPush(TestData.gitRef(force = true, forceUnknown = true))),
            TestData.gitPushView(TestData.gitPush(hidden)),
            TestData.askView(rule = forceRule),
        )
        red.forEach { v -> assertTrue(v.requestId, heldByDanger(v)) }
        assertFalse(heldByDanger(TestData.searchView()))
        assertFalse(heldByDanger(TestData.askView(rule = envRule)))
    }

    @Test
    fun `an ordinary request has no warnings`() {
        assertTrue(lines(TestData.searchView(), TestData.suggestion()).isEmpty())
        assertNull(dangerTag(TestData.searchView()))
    }
}
