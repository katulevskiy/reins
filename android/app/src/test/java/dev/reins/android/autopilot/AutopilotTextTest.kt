package dev.reins.android.autopilot

import dev.reins.android.TestData
import dev.reins.android.feedback.Event
import dev.reins.core.AutopilotMode
import dev.reins.core.AutopilotSettings
import dev.reins.core.ConnectionAutopilot
import dev.reins.core.ModelState
import dev.reins.core.OvernightRisky
import dev.reins.core.RiskLevel
import dev.reins.core.Verdict
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class AutopilotTextTest {
    private fun settings(
        bypass: Long? = null,
        connections: List<ConnectionAutopilot> = emptyList(),
        overnight: Long? = null,
        risky: OvernightRisky = OvernightRisky.WAIT,
    ) = AutopilotSettings(
        mode = when {
            bypass != null -> AutopilotMode.BYPASS
            overnight != null -> AutopilotMode.OVERNIGHT
            else -> AutopilotMode.MANUAL
        },
        baseMode = if (overnight != null) AutopilotMode.OVERNIGHT else AutopilotMode.MANUAL,
        bypassUntil = bypass,
        bypassSince = bypass?.let { 500L },
        overnightUntil = overnight,
        overnightSince = overnight?.let { 400L },
        overnightWhenRisky = risky,
        defaultProfileId = "personal",
        wifiOnly = true,
        model = TestData.modelStatus(ModelState.INSTALLED),
        connections = connections,
    )

    private fun conn(id: String, mode: AutopilotMode, profile: String = "personal", bypass: Long? = null, overnight: Long? = null) = ConnectionAutopilot(
        connectionId = id,
        baseMode = if (overnight != null) AutopilotMode.OVERNIGHT else null,
        bypassUntil = bypass,
        bypassSince = null,
        overnightUntil = overnight,
        overnightSince = overnight?.let { 300L },
        mode = mode,
        profileId = profile,
    )

    @Test
    fun `a profile icon that is a plain word shows the default emoji`() {
        // Accounts made earlier have the words "person" and "work" as the default profiles' icons.
        assertEquals("🙂", AutopilotText.profileIcon(TestData.profile(icon = "person")))
        assertEquals("💼", AutopilotText.profileIcon(TestData.profile(name = "Work", icon = "work")))
        assertEquals("🏠", AutopilotText.profileIcon(TestData.profile(icon = "🏠")))
        assertEquals("🙂", AutopilotText.profileIcon(TestData.profile(icon = " ")))
    }

    @Test
    fun `switching modes sounds like what it lets happen`() {
        assertEquals(Event.AutopilotOn, AutopilotText.modeChangeEvent(AutopilotMode.MANUAL, AutopilotMode.AUTO))
        assertEquals(Event.AutopilotOff, AutopilotText.modeChangeEvent(AutopilotMode.AUTO, AutopilotMode.MANUAL))
        assertEquals(Event.BypassOn, AutopilotText.modeChangeEvent(AutopilotMode.AUTO, AutopilotMode.BYPASS))
        assertEquals(Event.BypassOff, AutopilotText.modeChangeEvent(AutopilotMode.BYPASS, AutopilotMode.MANUAL))
        assertEquals(Event.LockdownOn, AutopilotText.modeChangeEvent(AutopilotMode.BYPASS, AutopilotMode.LOCKDOWN))
        assertEquals(Event.LockdownOff, AutopilotText.modeChangeEvent(AutopilotMode.LOCKDOWN, AutopilotMode.AUTO))
        assertNull(AutopilotText.modeChangeEvent(AutopilotMode.AUTO, AutopilotMode.AUTO))
    }

    @Test
    fun `the modes run from Manual to Lockdown and step by autonomy`() {
        assertEquals(listOf("Manual", "Auto", "Overnight", "Bypass", "Lockdown"), AutopilotText.modes.map(AutopilotText::name))
        assertEquals(
            listOf(AutopilotMode.LOCKDOWN, AutopilotMode.MANUAL, AutopilotMode.AUTO, AutopilotMode.OVERNIGHT, AutopilotMode.BYPASS),
            AutopilotText.byRisk,
        )
        assertEquals("Ask, with a hint", AutopilotText.line(AutopilotMode.MANUAL))
        assertTrue(AutopilotText.needsModel(AutopilotMode.AUTO))
        assertTrue("it decides like Auto", AutopilotText.needsModel(AutopilotMode.OVERNIGHT))
        assertTrue("Manual only goes without its hints", !AutopilotText.needsModel(AutopilotMode.MANUAL))
        assertTrue(AutopilotText.isTimed(AutopilotMode.OVERNIGHT) && AutopilotText.isTimed(AutopilotMode.BYPASS))
        assertTrue(!AutopilotText.isTimed(AutopilotMode.AUTO) && !AutopilotText.isTimed(null))
    }

    @Test
    fun `Overnight's name is one constant, and every mode has words`() {
        assertEquals(AutopilotText.OVERNIGHT, AutopilotText.name(AutopilotMode.OVERNIGHT))
        AutopilotMode.entries.forEach { m ->
            assertTrue(m.name, AutopilotText.name(m).isNotBlank() && AutopilotText.line(m).isNotBlank())
        }
        assertEquals("Turn on Overnight for every AI", AutopilotText.overnightAuthTitle(null))
        assertEquals("Turn on Overnight for Claude", AutopilotText.overnightAuthTitle("Claude"))
        assertEquals("Overnight for every AI?", AutopilotText.overnightTitle(null))
        assertTrue(AutopilotText.MODE_HELP.contains(AutopilotText.OVERNIGHT))
    }

    @Test
    fun `switching into and out of Overnight feels like more and then less autonomy`() {
        assertEquals(Event.AutopilotOn, AutopilotText.modeChangeEvent(AutopilotMode.AUTO, AutopilotMode.OVERNIGHT))
        assertEquals(Event.AutopilotOff, AutopilotText.modeChangeEvent(AutopilotMode.OVERNIGHT, AutopilotMode.AUTO))
        assertEquals(Event.BypassOn, AutopilotText.modeChangeEvent(AutopilotMode.OVERNIGHT, AutopilotMode.BYPASS))
    }

    @Test
    fun `the Overnight sheet says what it does and what waits, by the setting`() {
        assertEquals("Decides like Auto. Too risky waits.", AutopilotText.overnightLine(OvernightRisky.WAIT))
        assertEquals("Decides like Auto. Too risky is denied.", AutopilotText.overnightLine(OvernightRisky.DENY))
        assertEquals("If something is too risky:", AutopilotText.WHEN_RISKY)
        assertEquals(listOf("Wait for me", "Deny it"), AutopilotText.whenRiskyChoices.map(AutopilotText::whenRiskyLabel))
        assertEquals(OvernightRisky.WAIT, AutopilotText.whenRiskyChoices.first())
        assertTrue(AutopilotText.overnightStillAsks(OvernightRisky.DENY).contains("denied"))
    }

    @Test
    fun `without a model that rates risk, Overnight's help says it decides like Auto`() {
        assertTrue(AutopilotText.overnightHelp(ratesRisk = false).contains("decides like Auto"))
        assertTrue(AutopilotText.overnightHelp(ratesRisk = false).contains("doesn't rate risk"))
        assertTrue(AutopilotText.overnightHelp(ratesRisk = true).contains("weighed"))
        assertTrue(!AutopilotText.overnightHelp(ratesRisk = true).contains("doesn't rate risk"))
    }

    @Test
    fun `a risk reads as its level and why, and a held one says Overnight held it`() {
        assertEquals("Risk: severe · force push", AutopilotText.riskLine(TestData.risk()))
        assertEquals("Risk: catastrophic · deletes the default branch main", AutopilotText.riskLine(TestData.risk(RiskLevel.CATASTROPHIC, "deletes the default branch main")))
        assertEquals("Risk: low", AutopilotText.riskLine(TestData.risk(RiskLevel.BENIGN, "  ")))
        assertEquals("Risk: risky", AutopilotText.riskLine(TestData.risk(RiskLevel.RISKY, "")))
        assertNull(AutopilotText.heldLine(TestData.risk()))
        assertEquals("Held by Overnight", AutopilotText.heldLine(TestData.risk(held = true)))
        assertTrue(!AutopilotText.riskWorthListing(null))
        assertTrue("a low risk is not worth a line in a list", !AutopilotText.riskWorthListing(TestData.risk(RiskLevel.BENIGN)))
        assertTrue(AutopilotText.riskWorthListing(TestData.risk(RiskLevel.BENIGN, held = true)))
        assertTrue(AutopilotText.riskWorthListing(TestData.risk(RiskLevel.RISKY)))
    }

    @Test
    fun `the Overnight notice covers the global one and each AI's, while they are in force`() {
        assertNull(AutopilotText.overnightNotice(settings(), { it }, 1_000))
        val global = AutopilotText.overnightNotice(settings(overnight = 1_000 + 8 * 3_600), { it }, 1_000)!!
        assertEquals(AutopilotMode.OVERNIGHT, global.mode)
        assertTrue(global.global)
        assertEquals("Overnight on · 8 h left", global.title)
        assertEquals("For every AI. Too risky or unclear waits for you.", global.text)
        assertEquals(400L, global.since)
        val deny = AutopilotText.overnightNotice(settings(overnight = AutopilotText.UNTIL_OFF, risky = OvernightRisky.DENY), { it }, 1_000)!!
        assertEquals("Overnight on · until you stop it", deny.title)
        assertTrue(deny.text.endsWith("Too risky is denied. Unclear waits for you."))
        assertNull(deny.nextEnd)
        val one = AutopilotText.overnightNotice(
            settings(connections = listOf(conn("c1", AutopilotMode.OVERNIGHT, overnight = 1_000 + 600), conn("c2", AutopilotMode.BYPASS, overnight = 1_000 + 900))),
            { if (it == "c1") "Claude Code" else "?" },
            1_000,
        )!!
        assertEquals("a bypassed AI shows its bypass, not its Overnight", listOf("c1"), one.connectionIds)
        assertEquals("For Claude Code. Too risky or unclear waits for you.", one.text)
        assertEquals(1_600L, one.nextEnd)
        assertNull("no bypass notice for it", AutopilotText.bypassNotice(settings(overnight = 2_000), { it }, 1_000))
        assertEquals("Deciding while you're away", AutopilotText.runningLine(AutopilotMode.OVERNIGHT, 2_000, 1_000, 1_100))
        assertEquals("On until you stop it · 12 min so far", AutopilotText.runningLine(AutopilotMode.OVERNIGHT, AutopilotText.UNTIL_OFF, 0, 12 * 60 + 5))
    }

    @Test
    fun `time left reads in minutes, hours or days, and as a clock`() {
        assertEquals("42 min left", AutopilotText.timeLeft(1_000 + 41 * 60 + 5, 1_000))
        assertEquals("1 min left", AutopilotText.timeLeft(1_010, 1_000))
        assertEquals("5 h left", AutopilotText.timeLeft(1_000 + 4 * 3_600 + 10, 1_000))
        assertEquals("3 days left", AutopilotText.timeLeft(1_000 + 3 * 86_400, 1_000))
        assertEquals("14:05", AutopilotText.clock(1_000 + 14 * 60 + 5, 1_000))
        assertEquals("0:00", AutopilotText.clock(900, 1_000))
        assertEquals("5:04:09", AutopilotText.clock(1_000 + 5 * 3_600 + 4 * 60 + 9, 1_000))
        assertEquals("2d 4h", AutopilotText.clock(1_000 + 2 * 86_400 + 4 * 3_600 + 59, 1_000))
    }

    @Test
    fun `a bypass without an end shows no countdown, only how long it has run`() {
        val off = AutopilotText.UNTIL_OFF
        assertTrue(AutopilotText.isUntilOff(off))
        assertTrue(!AutopilotText.isUntilOff(1_000))
        assertEquals("until you stop it", AutopilotText.timeLeft(off, 1_000))
        assertEquals("∞", AutopilotText.clock(off, 1_000))
        assertEquals(1f, AutopilotText.bypassFraction(off, 0, 1_000))
        assertEquals("just started", AutopilotText.ranFor(1_000, 1_030))
        assertEquals("12 min so far", AutopilotText.ranFor(0, 12 * 60 + 5))
        assertEquals("5 h so far", AutopilotText.ranFor(0, 5 * 3_600 + 100))
        assertEquals("2 days so far", AutopilotText.ranFor(0, 2 * 86_400 + 100))
        assertEquals("On until you stop it · 2 days so far", AutopilotText.bypassLine(off, 0, 2 * 86_400 + 100))
        assertEquals("On until you stop it", AutopilotText.bypassLine(off, null, 5))
        assertEquals("Approving without asking", AutopilotText.bypassLine(1_600, 1_000, 1_100))
        val notice = AutopilotText.bypassNotice(settings(bypass = off), { it }, 1_000)!!
        assertEquals("Bypass on · until you stop it", notice.title)
        assertTrue(notice.untilOff)
        assertEquals(500L, notice.since)
        assertNull("nothing to wait for", notice.nextEnd)
    }

    @Test
    fun `the bypass ring measures from its start, or against the length most likely chosen`() {
        assertEquals(0.5f, AutopilotText.bypassFraction(2_000, 1_000, 1_500))
        assertEquals(0f, AutopilotText.bypassFraction(2_000, 1_000, 3_000))
        assertEquals(15 * 60L, AutopilotText.bypassLength(14 * 60))
        assertEquals(60 * 60L, AutopilotText.bypassLength(16 * 60))
        assertEquals(1_440 * 60L, AutopilotText.bypassLength(5 * 3_600))
    }

    @Test
    fun `bypass lengths read as people say them, within a year`() {
        assertEquals(listOf("15 min", "1 hour", "1 day"), AutopilotText.bypassMinutes.map(AutopilotText::lengthLabel))
        assertEquals(listOf("4 hours", "1 week"), AutopilotText.bypassMoreMinutes.map(AutopilotText::lengthLabel))
        assertEquals("3 days", AutopilotText.lengthLabel(AutopilotText.customMinutes(3, AutopilotText.BypassUnit.Days)))
        assertEquals("2 weeks", AutopilotText.lengthLabel(AutopilotText.customMinutes(2, AutopilotText.BypassUnit.Weeks)))
        assertEquals("7 min", AutopilotText.lengthLabel(AutopilotText.customMinutes(7, AutopilotText.BypassUnit.Minutes)))
        assertEquals("Until off", AutopilotText.lengthLabel(null))
        assertEquals(1u, AutopilotText.customMinutes(0, AutopilotText.BypassUnit.Minutes))
        assertEquals(52u * 10_080u, AutopilotText.customMinutes(99, AutopilotText.BypassUnit.Weeks))
        assertTrue(AutopilotText.customMinutes(99, AutopilotText.BypassUnit.Days) <= AutopilotText.MAX_BYPASS_MINUTES)
    }

    @Test
    fun `thresholds, replays, kinds and profiles read in a few words`() {
        val custom = TestData.profile(preset = dev.reins.core.Preset.CUSTOM, approveAt = 0.93f, denyAt = 0.88f)
        assertEquals("Approves when 93% sure, denies when 88% sure", AutopilotText.presetLine(custom))
        assertEquals(
            "Last 30 days at these: 41 decided for you, 12 asked, 0 you'd have undone.",
            AutopilotText.replayLine(dev.reins.core.ThresholdReplay(30u, 41u, 12u, 0u)),
        )
        assertEquals("94%", AutopilotText.share(0.94f))
        assertEquals("—", AutopilotText.share(null))
        assertEquals("On Auto · 26 decisions", AutopilotText.kindLine(TestData.classView(decisions = 26u, autoApprove = true, toUnlock = 0u)))
        assertEquals("8 more to unlock", AutopilotText.kindLine(TestData.classView(toUnlock = 8u)))
        assertEquals("Always asks", AutopilotText.kindLine(TestData.classView(manual = false)))
        val p = TestData.profile(memory = 140u)
        assertEquals("Claude Code, ChatGPT · 140 decisions", AutopilotText.profileLine(p, listOf("Claude Code", "ChatGPT")))
        assertEquals("1 decision", AutopilotText.profileLine(TestData.profile(memory = 1u), emptyList()))
        val s = settings(connections = listOf(conn("c2", AutopilotMode.MANUAL, "work")))
        val all = listOf("c1" to "Claude Code", "c2" to "ChatGPT", "c3" to "Cursor")
        assertEquals(listOf("Claude Code", "Cursor"), AutopilotText.profileAis(p, s, all))
        assertEquals(listOf("ChatGPT"), AutopilotText.profileAis(TestData.profile("work", isDefault = false, connections = listOf("c2")), s, all))
    }

    @Test
    fun `the bypass notice covers the global bypass and each connection's`() {
        assertNull(AutopilotText.bypassNotice(settings(), { it }, 1_000))
        assertNull("an ended bypass shows nothing", AutopilotText.bypassNotice(settings(bypass = 900), { it }, 1_000))
        val global = AutopilotText.bypassNotice(settings(bypass = 1_000 + 600), { it }, 1_000)!!
        assertTrue(global.global)
        assertEquals("Bypass on · 10 min left", global.title)
        assertEquals("For every AI. New connections and new phones still ask.", global.text)
        val one = AutopilotText.bypassNotice(
            settings(connections = listOf(conn("c1", AutopilotMode.BYPASS, bypass = 1_000 + 120))),
            { if (it == "c1") "Claude Code" else "?" },
            1_000,
        )!!
        assertTrue(one.text.contains("Claude Code"))
        assertEquals(listOf("c1"), one.connectionIds)
        assertEquals(1_120L, one.until)
    }

    @Test
    fun `automatic decisions are titled by who decided`() {
        assertEquals("Autopilot approved", AutopilotText.decisionTitle(TestData.autoDecision()))
        assertEquals("Autopilot denied", AutopilotText.decisionTitle(TestData.autoDecision(verdict = Verdict.DENY)))
        assertEquals("Approved by Bypass", AutopilotText.decisionTitle(TestData.autoDecision(decidedBy = "bypass")))
        assertEquals("Denied by Lockdown", AutopilotText.decisionTitle(TestData.autoDecision(verdict = Verdict.DENY, decidedBy = "lockdown")))
        assertEquals("Approved by Overnight", AutopilotText.decisionTitle(TestData.autoDecision(decidedBy = "overnight")))
        assertEquals("Denied by Overnight", AutopilotText.decisionTitle(TestData.autoDecision(verdict = Verdict.DENY, decidedBy = "overnight")))
        assertEquals("91% sure", AutopilotText.decisionDetail(TestData.autoDecision(decidedBy = "overnight")))
        assertEquals("91% sure", AutopilotText.decisionDetail(TestData.autoDecision()))
        assertNull(AutopilotText.decisionDetail(TestData.autoDecision(decidedBy = "bypass")))
    }

    @Test
    fun `suggestions say what Autopilot would do and why it holds back`() {
        assertEquals("Autopilot would approve · 97%", AutopilotText.suggestionHeadline(TestData.suggestion()))
        assertEquals("Autopilot would deny · 90%", AutopilotText.suggestionHeadline(TestData.suggestion(verdict = Verdict.DENY, pDeny = 0.9f)))
        assertEquals("Autopilot always asks you for this", AutopilotText.suggestionHeadline(TestData.suggestion(floor = true, judged = false)))
        assertTrue(AutopilotText.suggestionNotes(TestData.suggestion(novel = true)).single().contains("never approved"))
        assertTrue(AutopilotText.suggestionNotes(TestData.suggestion(judged = false, reason = "No model on this phone")).contains("No model on this phone"))
    }

    @Test
    fun `a class fills its ring with answers and is full once it runs by itself`() {
        assertEquals(0.6f, AutopilotText.unlockProgress(TestData.classView(decisions = 12u, toUnlock = 8u)), 0.001f)
        assertEquals(1f, AutopilotText.unlockProgress(TestData.classView(autoApprove = true, toUnlock = 0u)), 0f)
        assertEquals("8 more decisions to unlock", AutopilotText.classStatus(TestData.classView()))
        assertEquals("Locked by you · always asks", AutopilotText.classStatus(TestData.classView(manual = false)))
        assertEquals("Approves on its own", AutopilotText.classStatus(TestData.classView(autoApprove = true, toUnlock = 0u)))
        assertEquals("11 approved · 1 denied · 96% accurate", AutopilotText.classNumbers(TestData.classView(accuracy = 0.96f)))
    }

    @Test
    fun `a model that fails its check says so plainly`() {
        assertTrue(AutopilotText.modelError("sha-256 of model.onnx does not match").contains("did not match"))
        assertEquals("Connection reset.", AutopilotText.modelError("connection reset"))
        assertTrue(AutopilotText.modelError(null).contains("Try again"))
    }

    @Test
    fun `the model card states sizes in megabytes and knows the base checkpoint's range`() {
        assertEquals("about 300–450 MB", AutopilotText.modelSize(TestData.modelStatus()))
        val downloading = TestData.modelStatus(ModelState.DOWNLOADING, downloaded = 103_000_000u, size = 412_000_000u)
        assertEquals("Downloading · 103 MB of 412 MB", AutopilotText.modelState(downloading, waitingForNetwork = false, wifiOnly = true))
        assertEquals(0.25f, AutopilotText.fraction(downloading)!!, 0.001f)
        assertEquals("Waiting for Wi-Fi", AutopilotText.modelState(TestData.modelStatus(), waitingForNetwork = true, wifiOnly = true))
    }

    @Test
    fun `the examples use the situation format`() {
        AutopilotText.examples.forEach { e ->
            assertTrue(e.title, e.situation.startsWith("connection: "))
            assertTrue(e.title, e.situation.lines().all { it.contains(": ") || it == "--- written by the AI ---" })
        }
    }
}
